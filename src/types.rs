use btleplug::api::{
    CharPropFlags, Characteristic, Peripheral as _, PeripheralProperties, Service, WriteType, bleuuid::BleUuid,
};
use btleplug::platform::Peripheral;

use serde::Serialize;
use serde_json::Value;
use tokio::time::timeout;
use uuid::Uuid;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::characteristic_data::CharFormat;
use crate::util::{format_properties, serialize_char_props, serialize_hex, serialize_hex_option};
use crate::{CHARACTERISTIC_MAP, DESCRIPTOR_MAP, SERVICE_MAP};
use crate::{CONNECT_TIMEOUT, DISCONNECT_TIMEOUT, ENUMERATE_TIMEOUT, WRITE_TIMEOUT};

#[derive(Debug, Clone, Serialize)]
pub struct DeviceInfo {
    pub id: String, // UUID on MacOS but MAC accress on Win/Linux,
    pub name: Option<String>,
    pub rssi: Option<i16>,
    pub services: BTreeMap<Uuid, ServiceInfo>,
}

impl DeviceInfo {
    /// Create device from advertisment
    pub async fn new(p: &Peripheral) -> anyhow::Result<Self> {
        let properties = p.properties().await?.unwrap_or_default();
        let id = p.id().to_string();
        let name = properties.local_name;
        let rssi = properties.rssi;
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
                        uuid,
                        service_type: SERVICE_MAP.get(&uuid).map(|v| &**v),
                        characteristics: BTreeMap::new(),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        Ok(DeviceInfo {
            id,
            name,
            rssi,
            services,
        })
    }

    /// Enumerate device (with filters)
    pub async fn enumerate(
        &mut self,
        peripheral: &Peripheral,
        service_filter: &HashSet<Uuid>,
        characteristic_filter: &HashSet<Uuid>,
    ) -> anyhow::Result<()> {
        // Clear existing servcices (from advertisment)
        self.services.clear();
        // Discover services
        match timeout(
            Duration::from_secs(ENUMERATE_TIMEOUT.load(Ordering::Relaxed)),
            peripheral.discover_services(),
        )
        .await
        {
            Ok(Ok(_)) => {
                for service in peripheral.services() {
                    if service_filter.is_empty() || service_filter.contains(&service.uuid) {
                        for characteristic in &service.characteristics {
                            if characteristic_filter.is_empty() || characteristic_filter.contains(&characteristic.uuid)
                            {
                                self.services
                                    .entry(service.uuid)
                                    .or_insert(ServiceInfo::new(&service))
                                    .characteristics
                                    .insert(characteristic.uuid, CharacteristicInfo::new(characteristic));
                            }
                        }
                    }
                }
            }
            _ => {
                anyhow::bail!("Service discovery failed/timeout for {}", peripheral.id())
            }
        }
        Ok(())
    }

    /// Connect to device (note that you need to manage connect/disconnect explicitly)
    pub async fn connect(&self, peripheral: &Peripheral) -> anyhow::Result<()> {
        if !peripheral.is_connected().await? {
            match timeout(
                Duration::from_secs(CONNECT_TIMEOUT.load(Ordering::Relaxed)),
                peripheral.connect(),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(e)) => anyhow::bail!("Connect error for {}: {e}", peripheral.id()),
                Err(_elapsed) => {
                    // Dropping the connect future does NOT stop CoreBluetooth from
                    // retrying; cancel it explicitly or it wedges the central.
                    let _ = peripheral.disconnect().await;
                    anyhow::bail!("Connect timeout for {}", peripheral.id());
                }
            }
        }
        Ok(())
    }

    /// Disconnect from device (note that you need to manage connect/disconnect explicitly)
    pub async fn disconnect(&self, peripheral: &Peripheral) -> anyhow::Result<()> {
        if peripheral.is_connected().await?
            && let Err(e) = timeout(
                Duration::from_secs(DISCONNECT_TIMEOUT.load(Ordering::Relaxed)),
                peripheral.disconnect(),
            )
            .await
        {
            anyhow::bail!("Disconnect timeout/error for {}: {:?}", peripheral.id(), e);
        }
        Ok(())
    }

    /// Update RSSI
    pub async fn update_rssi(&mut self, peripheral: &Peripheral) {
        if let Ok(Some(PeripheralProperties { rssi, .. })) = peripheral.properties().await {
            self.rssi = rssi;
        }
    }

    /// Read data for filtered characteristics
    pub async fn read(
        &mut self,
        peripheral: &Peripheral,
        decode_map: &HashMap<Uuid, CharFormat>,
    ) -> anyhow::Result<()> {
        for service in self.services.values_mut() {
            for characteristic in service.characteristics.values_mut() {
                if characteristic.read(peripheral, decode_map).await.is_err() {
                    anyhow::bail!("Error reading characteristic data: {}", characteristic.uuid);
                }
            }
        }
        self.update_rssi(peripheral).await;
        Ok(())
    }

    /// Subscribe to filtered characteristics
    pub async fn subscribe(&mut self, peripheral: &Peripheral) -> anyhow::Result<Vec<SubscriptionInfo>> {
        let mut result = Vec::new();
        for service in self.services.values_mut() {
            for characteristic in service.characteristics.values_mut() {
                match characteristic.subscribe(peripheral).await {
                    Ok(Some(s)) => result.push(s),
                    Ok(None) => {}
                    Err(e) => anyhow::bail!("Error subscribing: {} {}", characteristic.uuid, e),
                }
            }
        }
        Ok(result)
    }
}

impl std::fmt::Display for DeviceInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "{} | {} | {} dBm",
            self.id,
            match self.name {
                Some(ref s) => s.as_str(),
                None => "Unknown",
            },
            self.rssi.unwrap_or(0)
        )?;
        for s in &self.services {
            write!(f, "{}", s.1)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceInfo {
    pub uuid: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_type: Option<&'static str>,
    pub characteristics: BTreeMap<Uuid, CharacteristicInfo>,
}

impl ServiceInfo {
    pub fn new(s: &Service) -> Self {
        Self {
            uuid: s.uuid,
            service_type: SERVICE_MAP.get(&s.uuid).copied(),
            characteristics: BTreeMap::new(),
        }
    }
}

impl std::fmt::Display for ServiceInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.characteristics.is_empty() {
            writeln!(
                f,
                "    └─ Service: {} {}",
                self.uuid.to_short_string(),
                if let Some(t) = self.service_type {
                    format!(" [{}] ", t)
                } else {
                    "".to_string()
                },
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
        }
        for c in self.characteristics.values() {
            write!(f, "{}", c)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CharacteristicInfo {
    pub uuid: Uuid,
    pub service_uuid: Uuid,
    #[serde(serialize_with = "serialize_char_props")]
    pub properties: CharPropFlags,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub characteristic_type: Option<&'static str>,
    #[serde(serialize_with = "serialize_hex_option")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Vec<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoded: Option<Value>,
    pub descriptors: Vec<DescriptorInfo>,
    // Store the underlying characteristic struct for read/write/notify
    #[serde(skip)]
    characteristic: Characteristic,
}

impl CharacteristicInfo {
    pub fn new(c: &Characteristic) -> Self {
        Self {
            uuid: c.uuid,
            service_uuid: c.service_uuid,
            properties: c.properties,
            characteristic_type: CHARACTERISTIC_MAP.get(&c.uuid).map(|v| &**v),
            value: None,
            decoded: None,
            descriptors: c.descriptors.iter().map(|d| DescriptorInfo::new(&d.uuid)).collect(),
            characteristic: c.clone(),
        }
    }
    pub async fn read(&mut self, p: &Peripheral, decode_map: &HashMap<Uuid, CharFormat>) -> anyhow::Result<()> {
        if self.properties.contains(CharPropFlags::READ) {
            self.value = Some(p.read(&self.characteristic).await?);
            self.decoded = self
                .value
                .as_ref()
                .and_then(|v| decode_map.get(&self.uuid).and_then(|fmt| fmt.decode(v).ok()))
        }
        Ok(())
    }
    pub async fn write(&mut self, p: &Peripheral, without_response: bool, value: &[u8]) -> anyhow::Result<()> {
        if self.properties.contains(CharPropFlags::WRITE)
            || self.properties.contains(CharPropFlags::WRITE_WITHOUT_RESPONSE)
        {
            match timeout(
                Duration::from_secs(WRITE_TIMEOUT.load(Ordering::Relaxed)),
                p.write(
                    &self.characteristic,
                    value,
                    if self.properties.contains(CharPropFlags::WRITE_WITHOUT_RESPONSE) || without_response {
                        WriteType::WithoutResponse
                    } else {
                        WriteType::WithResponse
                    },
                ),
            )
            .await
            {
                Ok(Ok(_)) => Ok(()),
                Ok(Err(e)) => anyhow::bail!("Write Error: {} -> {}", self.uuid, e),
                Err(_) => anyhow::bail!("Write Timeout: {}", self.uuid),
            }
        } else {
            anyhow::bail!("Characteristic not writeable: {}", self.uuid)
        }
    }
    pub async fn subscribe(&mut self, peripheral: &Peripheral) -> anyhow::Result<Option<SubscriptionInfo>> {
        if self.properties.contains(CharPropFlags::NOTIFY) || self.properties.contains(CharPropFlags::INDICATE) {
            peripheral.subscribe(&self.characteristic).await?;
            Ok(Some(SubscriptionInfo {
                device: peripheral.id().to_string(),
                service: self.service_uuid,
                characteristic: self.uuid,
            }))
        } else {
            Ok(None)
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
                    .join(",")
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

#[derive(Debug, Clone, Serialize)]
pub struct SubscriptionInfo {
    pub device: String, // Linux returns hci path rather than UUID
    pub service: Uuid,
    pub characteristic: Uuid,
}

impl std::fmt::Display for SubscriptionInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Subscribed :: Device: {}\n              └─ Service: {}\n                  └─ Characteristic: {}",
            self.device, self.service, self.characteristic
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DescriptorInfo {
    pub uuid: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(skip_deserializing)]
    pub descriptor_type: Option<&'static str>,
}

impl DescriptorInfo {
    pub fn new(uuid: &Uuid) -> Self {
        Self {
            uuid: *uuid,
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

#[derive(Debug, Clone, Serialize)]
pub struct NotificationData {
    pub service: Uuid,
    pub characteristic: Uuid,
    #[serde(serialize_with = "serialize_hex")]
    pub value: Vec<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoded: Option<Value>,
}

impl std::fmt::Display for NotificationData {
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
