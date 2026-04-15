use btleplug::api::{Peripheral as _, bleuuid::BleUuid};
use btleplug::platform::Peripheral;

use hex;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::SERVICE_MAP;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub rssi: i16,
    pub services: Vec<ServiceInfo>,
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
            .map(|uuid| ServiceInfo {
                uuid: uuid.to_short_string(),
                service_type: SERVICE_MAP.get(uuid).map(|v| &**v),
                characteristics: Vec::new(),
            })
            .collect::<Vec<_>>();
        Ok(DeviceInfo {
            id: id.to_string(),
            name: name,
            rssi,
            services,
        })
    }
}

impl std::fmt::Display for DeviceInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{} | {} | {} dBm", self.id, self.name, self.rssi)?;
        for s in &self.services {
            write!(f, "{}", s)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInfo {
    pub uuid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(skip_deserializing)]
    pub service_type: Option<&'static str>,
    pub characteristics: Vec<CharacteristicInfo>,
}

impl std::fmt::Display for ServiceInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.characteristics.is_empty() {
            writeln!(f, "    Service: {}", self.uuid,)?;
        } else {
            writeln!(
                f,
                "    Service: {} {}({} characteristics)",
                self.uuid,
                if let Some(t) = self.service_type {
                    format!(" [{}] ", t)
                } else {
                    "".to_string()
                },
                self.characteristics.len()
            )?;
            for c in &self.characteristics {
                write!(f, "{}", c)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacteristicInfo {
    pub uuid: String,
    pub properties: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(skip_deserializing)]
    pub char_type: Option<&'static str>,
    #[serde(serialize_with = "serialize_hex_option", deserialize_with = "deserialize_hex_option")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Vec<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoded: Option<String>,
}

impl std::fmt::Display for CharacteristicInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "    └─ Characteristic: {} {}", self.uuid, self.properties)?;
        if let Some(t) = self.char_type {
            writeln!(f, "       Type: {}", t)?;
        }
        if let Some(ref value) = self.value {
            writeln!(f, "       Value: 0x{}", hex::encode(value))?;
        }
        if let Some(ref decoded) = self.decoded {
            writeln!(f, "       Decoded: {}", decoded)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationInfo {
    pub service: String,
    pub characteristic: String,
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
                self.service,
                self.characteristic,
                hex::encode(&self.value),
                decoded
            )?;
        } else {
            write!(
                f,
                "Notification >> Service: {}\n                └─ Characteristic: {}\n                   Value: 0x{}",
                self.service,
                self.characteristic,
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
