use btleplug::api::{bleuuid::BleUuid, CharPropFlags, Peripheral as _};
use btleplug::platform::Peripheral;
use std::collections::{BTreeSet, HashSet};
use std::time::Duration;
use tokio::time::timeout;
use uuid::Uuid;

use crate::device_info::{CharacteristicInfo, DeviceInfo, ServiceInfo};
use crate::util::format_properties;
use crate::{CHARACTERISTIC_MAP, SERVICE_MAP};
use crate::{CONNECT_TIMEOUT, DISCONNECT_TIMEOUT, ENUMERATE_TIMEOUT};

pub async fn device_info(p: &Peripheral) -> anyhow::Result<DeviceInfo> {
    let properties = p.properties().await?.unwrap_or_default();
    let id = p.id();
    let name = properties
        .local_name
        .clone()
        .unwrap_or_else(|| "Unknown".to_string());
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

// Distinguish between no services (empty Some<Vec>) and no filter matches (None)
pub async fn enumerate_services(
    p: &Peripheral,
    read: bool,
    service_filter: &HashSet<Uuid>,
    characteristic_filter: &HashSet<Uuid>,
) -> anyhow::Result<Option<Vec<ServiceInfo>>> {
    let mut service_info = Vec::new();
    match timeout(Duration::from_secs(CONNECT_TIMEOUT), p.connect()).await {
        Ok(Ok(_)) => {
            match timeout(
                Duration::from_secs(ENUMERATE_TIMEOUT),
                p.discover_services(),
            )
            .await
            {
                Ok(Ok(_)) => {
                    let services = if service_filter.is_empty() {
                        p.services()
                    } else {
                        p.services()
                            .into_iter()
                            .filter(|s| service_filter.contains(&s.uuid))
                            .collect::<BTreeSet<_>>()
                    };
                    for service in services {
                        let mut chars = Vec::new();
                        for char in &service.characteristics {
                            if characteristic_filter.is_empty()
                                || characteristic_filter.contains(&char.uuid)
                            {
                                chars.push(CharacteristicInfo {
                                    uuid: char.uuid.to_short_string(),
                                    properties: format_properties(char.properties),
                                    char_type: CHARACTERISTIC_MAP.get(&char.uuid).map(|v| &**v),
                                    value: if read && char.properties.contains(CharPropFlags::READ)
                                    {
                                        p.read(char).await.ok()
                                    } else {
                                        None
                                    },
                                });
                            }
                        }
                        service_info.push(ServiceInfo {
                            uuid: service.uuid.to_short_string(),
                            service_type: SERVICE_MAP.get(&service.uuid).map(|v| &**v),
                            characteristics: chars,
                        });
                    }
                }
                _ => {
                    // eprintln!("Service discovery failed/timeout for {}", p.id())
                }
            }
            if let Err(_e) = timeout(Duration::from_secs(DISCONNECT_TIMEOUT), p.disconnect()).await
            {
                // eprintln!("Disconnect timeout/error for {}: {:?}", p.id(), e);
            }
        }
        _ => {
            // eprintln!("Connect timeout/error for {}", p.id())
        }
    }
    if service_info.is_empty() && !service_filter.is_empty() {
        Ok(None)
    } else {
        Ok(Some(service_info))
    }
}
