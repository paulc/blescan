use btleplug::api::{
    CharPropFlags, Characteristic, Descriptor, Peripheral as _, PeripheralProperties, Service, bleuuid::BleUuid,
};
use btleplug::platform::Peripheral;

use hex;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeSet, HashMap};
use uuid::Uuid;

use crate::characteristic_data::CharFormat;
use crate::util::format_properties;
use crate::{CHARACTERISTIC_MAP, DESCRIPTOR_MAP, SERVICE_MAP};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub rssi: i16,
    pub services: HashMap<Uuid, ServiceInfo>,
}

impl DeviceInfo {
    pub async fn new(p: &Peripheral) -> anyhow::Result<Self> {
        let properties = p.properties().await?.unwrap_or_default();
        let id = p.id();
        let name = properties.local_name.clone().unwrap_or_else(|| "Unknown".to_string());
        let rssi = properties.rssi.unwrap_or(0);
        // Read basic service data from the advertisment
        // but this may miss some services (only advertised
        // intermittently)
        let services = properties
            .services
            .iter()
            .map(|&uuid| {
                (
                    uuid,
                    ServiceInfo {
                        uuid: uuid,
                        service_type: SERVICE_MAP.get(&uuid).map(|v| &**v),
                        characteristics: HashMap::new(),
                    },
                )
            })
            .collect::<HashMap<_, _>>();
        Ok(DeviceInfo {
            id: id.to_string(),
            name: name,
            rssi,
            services,
        })
    }
    pub async fn update_rssi(&mut self, p: &Peripheral) {
        if let Ok(Some(PeripheralProperties { rssi: Some(rssi), .. })) = p.properties().await {
            self.rssi = rssi;
        }
    }
}

impl std::fmt::Display for DeviceInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{} | {} | {} dBm", self.id, self.name, self.rssi)?;
        for s in &self.services {
            write!(f, "{}", s.1)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInfo {
    pub uuid: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(skip_deserializing)]
    pub service_type: Option<&'static str>,
    pub characteristics: HashMap<Uuid, CharacteristicInfo>,
}

impl ServiceInfo {
    pub fn new(s: &Service) -> Self {
        Self {
            uuid: s.uuid.clone(),
            service_type: SERVICE_MAP.get(&s.uuid).map(|&v| &*v),
            characteristics: HashMap::new(),
        }
    }
}

impl std::fmt::Display for ServiceInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.characteristics.is_empty() {
            writeln!(
                f,
                "    └─ Service: {} {}({} characteristics)",
                self.uuid.to_short_string(),
                if let Some(t) = self.service_type {
                    format!(" [{}] ", t)
                } else {
                    "".to_string()
                },
                self.characteristics.len()
            )?;
        } else {
            writeln!(
                f,
                "    └─ Service: {} {}({} characteristics)",
                self.uuid.to_short_string(),
                if let Some(t) = self.service_type {
                    format!(" [{}] ", t)
                } else {
                    "".to_string()
                },
                self.characteristics.len()
            )?;
            for c in &self.characteristics {
                write!(f, "{}", c.1)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacteristicInfo {
    pub uuid: Uuid,
    pub service_uuid: Uuid,
    #[serde(skip_deserializing)]
    #[serde(serialize_with = "serialize_char_props")]
    pub properties: CharPropFlags,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(skip_deserializing)]
    pub characteristic_type: Option<&'static str>,
    #[serde(serialize_with = "serialize_hex_option", deserialize_with = "deserialize_hex_option")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Vec<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoded: Option<String>,
    pub descriptors: Vec<DescriptorInfo>,
}

impl CharacteristicInfo {
    pub fn new(c: &Characteristic) -> Self {
        Self {
            uuid: c.uuid.clone(),
            service_uuid: c.service_uuid.clone(),
            properties: c.properties.clone(),
            characteristic_type: CHARACTERISTIC_MAP.get(&c.uuid).map(|v| &**v),
            value: None,
            decoded: None,
            descriptors: c.descriptors.iter().map(|d| DescriptorInfo::new(&d.uuid)).collect(),
        }
    }
    pub async fn read(&mut self, p: &Peripheral, map: &HashMap<Uuid, CharFormat>) -> () {
        if self.properties.contains(CharPropFlags::READ) {
            self.value = p.read(&self.to_characteristic()).await.ok();
            self.decoded = self
                .value
                .as_ref()
                .and_then(|v| map.get(&self.uuid).map(|fmt| fmt.decode(v)));
        }
    }
    fn to_characteristic(&self) -> Characteristic {
        Characteristic {
            uuid: self.uuid.clone(),
            service_uuid: self.service_uuid.clone(),
            properties: self.properties.clone(),
            descriptors: self
                .descriptors
                .iter()
                .map(|d| Descriptor {
                    uuid: d.uuid.clone(),
                    service_uuid: self.service_uuid.clone(),
                    characteristic_uuid: self.uuid.clone(),
                })
                .collect::<BTreeSet<_>>(),
        }
    }
}

impl std::fmt::Display for CharacteristicInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "       └─ Characteristic: {} {}",
            self.uuid.to_short_string(),
            format_properties(&self.properties)
        )?;
        if !self.descriptors.is_empty() {
            writeln!(
                f,
                "          Descriptors: {}",
                self.descriptors
                    .iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join(",".into())
            )?;
        }
        if let Some(t) = self.characteristic_type {
            writeln!(f, "          Type: {}", t)?;
        }
        if let Some(ref value) = self.value {
            writeln!(f, "          Value: 0x{}", hex::encode(value))?;
        }
        if let Some(ref decoded) = self.decoded {
            writeln!(f, "          Decoded: {}", decoded)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DescriptorInfo {
    pub uuid: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(skip_deserializing)]
    pub descriptor_type: Option<&'static str>,
}

impl DescriptorInfo {
    pub fn new(uuid: &Uuid) -> Self {
        Self {
            uuid: uuid.clone(),
            descriptor_type: DESCRIPTOR_MAP.get(uuid).map(|v| &**v),
        }
    }
}
impl std::fmt::Display for DescriptorInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(t) = self.descriptor_type {
            write!(f, "{} [{}]", self.uuid.to_short_string(), t)?;
        } else {
            write!(f, "{}", self.uuid.to_short_string())?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationInfo {
    pub service: Uuid,
    pub characteristic: Uuid,
    #[serde(serialize_with = "serialize_hex", deserialize_with = "deserialize_hex")]
    pub value: Vec<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoded: Option<String>,
}

impl std::fmt::Display for NotificationInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(ref decoded) = self.decoded {
            write!(
                f,
                "Notification >> Service: {}\n                └─ Characteristic: {}\n                   Value: 0x{}\n                   Decoded: {}",
                self.service.to_short_string(),
                self.characteristic.to_short_string(),
                hex::encode(&self.value),
                decoded
            )?;
        } else {
            write!(
                f,
                "Notification >> Service: {}\n                └─ Characteristic: {}\n                   Value: 0x{}",
                self.service.to_short_string(),
                self.characteristic.to_short_string(),
                hex::encode(&self.value)
            )?;
        }
        Ok(())
    }
}

fn serialize_hex<S>(bytes: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&format!("0x{}", hex::encode(bytes)))
}

fn deserialize_hex<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    let s = s.strip_prefix("0x").unwrap_or(&s); // Strip 0x
    hex::decode(s).map_err(serde::de::Error::custom)
}

fn serialize_char_props<S>(props: &CharPropFlags, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&format_properties(props))
}

fn serialize_hex_option<S>(bytes: &Option<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match bytes {
        Some(v) => {
            let hex = format!("0x{}", hex::encode(v));
            serializer.serialize_str(&hex)
        }
        None => serializer.serialize_none(),
    }
}

fn deserialize_hex_option<'de, D>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<String> = Option::deserialize(deserializer)?;
    match opt {
        Some(s) => {
            let s = s.strip_prefix("0x").unwrap_or(&s); // Strip 0x
            hex::decode(s).map(Some).map_err(serde::de::Error::custom)
        }
        None => Ok(None),
    }
}
