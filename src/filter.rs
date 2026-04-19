use btleplug::api::{Characteristic, Peripheral as _, Service};
use btleplug::platform::Peripheral;
use regex::Regex;
use std::time::Duration;
use tokio::time::timeout;
use uuid::Uuid;

use std::collections::HashSet;

use crate::types::DeviceInfo;
use crate::{CONNECT_TIMEOUT, DISCONNECT_TIMEOUT, ENUMERATE_TIMEOUT};

/// Filter against basic device info
pub fn device_match(
    device: &DeviceInfo,
    rssi_filter: &Option<i16>,
    name_filter: &Vec<Regex>,
    id_filter: &Vec<String>,
) -> bool {
    rssi_filter.is_none_or(|rssi| device.rssi >= rssi)
        && (name_filter.is_empty() || name_filter.iter().any(|r| r.is_match(&device.name)))
        && (id_filter.is_empty() || id_filter.iter().any(|id| device.id == *id))
}

/// Callback based filter - mass match_callback & completed_callback
///
/// Example match callback (add matching characteristics to device):
///
///     let match_callback = {
///         let device = Arc::clone(&device);
///         let decode_map = Arc::clone(&decode_map);
///         async move |peripheral: &Peripheral,
///                     service: &Service,
///                     characteristic: &Characteristic|
///                     -> anyhow::Result<()> {
///             let mut device = device.lock().await;
///             let mut c = CharacteristicInfo::new(&characteristic);
///             if args.read {
///                 c.read(peripheral, &decode_map).await;
///             }
///             device
///                 .services
///                 .entry(service.uuid.clone())
///                 .or_insert(ServiceInfo::new(&service))
///                 .characteristics
///                 .insert(characteristic.uuid.clone(), c);
///             Ok(())
///         }
///     };
///
/// Example completed callback (prints matched data):
///
///     let completed_callback = {
///         let device = Arc::clone(&device);
///         let match_count = Arc::clone(&match_count);
///         async move |_peripheral: &Peripheral| -> anyhow::Result<()> {
///             let device = device.lock().await;
///             if !is_filtered || !device.services.is_empty() {
///                 if args.json {
///                     println!("{}", serde_json::to_string(&*device)?);
///                 } else {
///                     print!("[+] Device: {}", device);
///                 }
///                 match_count.fetch_add(1, Ordering::Relaxed);
///             }
///             Ok(())
///         }
///     };

pub async fn filter<MF, CF>(
    peripheral: &Peripheral,
    service_filter: &HashSet<Uuid>,
    characteristic_filter: &HashSet<Uuid>,
    match_callback: MF,
    completed_callback: CF,
) -> anyhow::Result<()>
where
    MF: AsyncFn(&Peripheral, &Service, &Characteristic) -> anyhow::Result<()>,
    CF: AsyncFn(&Peripheral) -> anyhow::Result<()>,
{
    match timeout(Duration::from_secs(CONNECT_TIMEOUT), peripheral.connect()).await {
        Ok(Ok(_)) => {
            match timeout(Duration::from_secs(ENUMERATE_TIMEOUT), peripheral.discover_services()).await {
                Ok(Ok(_)) => {
                    for service in peripheral.services() {
                        if service_filter.is_empty() || service_filter.contains(&service.uuid) {
                            for characteristic in &service.characteristics {
                                if characteristic_filter.is_empty()
                                    || characteristic_filter.contains(&characteristic.uuid)
                                {
                                    // Matched
                                    match_callback(peripheral, &service, characteristic).await?;
                                }
                            }
                        }
                    }
                    completed_callback(peripheral).await?;
                }
                _ => {
                    // eprintln!("Service discovery failed/timeout for {}", peripheral.id())
                }
            }
            if let Err(_e) = timeout(Duration::from_secs(DISCONNECT_TIMEOUT), peripheral.disconnect()).await {
                // eprintln!("Disconnect timeout/error for {}: {:?}", p.id(), e);
            }
        }
        _ => {
            // eprintln!("Connect timeout/error for {}", peripheral.id())
        }
    }
    Ok(())
}
