use anyhow::{Context, anyhow};
use btleplug::api::{Central, CentralEvent, CharPropFlags, Peripheral as _, ScanFilter, bleuuid::BleUuid};
use btleplug::platform::{Adapter, Peripheral};
use futures::StreamExt;
use std::time::Duration;
use tokio::time::timeout;
use uuid::Uuid;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use crate::PollArgs;
use crate::char_data::CharFormat;
use crate::types::{CharacteristicInfo, DeviceInfo, ServiceInfo};
use crate::util::{format_properties, parse_decoder, parse_uuid, uuid_filter};
use crate::{CHARACTERISTIC_MAP, SERVICE_MAP};
use crate::{CONNECT_TIMEOUT, ENUMERATE_TIMEOUT};

pub async fn run(central: Adapter, args: PollArgs) -> anyhow::Result<()> {
    let service_filter = uuid_filter(&args.service)?;
    let characteristic_filter = uuid_filter(&args.characteristic)?;
    let decode_map = parse_decoder(&args.decode)?;

    // Validate device uuids
    args.device
        .iter()
        .map(|s| parse_uuid(s))
        .collect::<Result<Vec<_>, _>>()
        .context("Error Parsing Device UUID")?;

    // ScanFilter only checks for services in the Advertisement
    // payload which tend to be very limited (31 bytes max) rather
    // then the full list of GATT services (which need connection)
    central.start_scan(ScanFilter::default()).await?;
    let mut seen = HashSet::new();

    let scan = async {
        let mut events = central.events().await?;
        while let Some(event) = events.next().await {
            // Check if the event is a device discovery
            match event {
                CentralEvent::DeviceDiscovered(id) => {
                    if seen.contains(&id) {
                        continue;
                    }
                    seen.insert(id.clone());
                    match central.peripheral(&id).await {
                        Ok(peripheral) => {
                            // Get basic info first (fast)
                            let device = DeviceInfo::new(&peripheral).await?;

                            // Filter by RSSI
                            if let Some(rssi) = args.rssi {
                                if device.rssi < rssi {
                                    continue;
                                }
                            }
                            // Filter by name
                            if !args.name.is_empty() && !args.name.contains(&device.name) {
                                continue;
                            }

                            // Filter by device UUID
                            if !args.device.is_empty() && !args.device.contains(&device.id) {
                                continue;
                            }

                            // Poll device in background
                            let service_filter = Arc::clone(&service_filter);
                            let characteristic_filter = Arc::clone(&characteristic_filter);
                            let decode_map = Arc::clone(&decode_map);

                            tokio::spawn(async move {
                                poll(
                                    &peripheral,
                                    &device,
                                    &service_filter,
                                    &characteristic_filter,
                                    args.interval,
                                    args.json,
                                    &decode_map,
                                )
                                .await?;
                                Ok::<(), anyhow::Error>(())
                            });
                        }
                        Err(e) => {
                            eprintln!("Error retrieving peripheral: {:?}", e);
                        }
                    }
                }
                _ => {
                    // eprintln!(">> EVENT: {:?}", event);
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    };

    if let Some(t) = args.timeout {
        if !args.json {
            println!("Listening for BLE advertisements: Timeout {t} secs");
        }
        match timeout(Duration::from_secs(t), scan).await {
            Ok(result) => result.map_err(|e| anyhow!("Scan Error: {e}"))?,
            Err(_) => println!("\n[!] Timeout reached. Stopping scan."),
        }
    } else {
        if !args.json {
            println!("Listening for BLE advertisements: Ctrl+C to stop");
        }
        scan.await.map_err(|e| anyhow!("Scan Error: {e}"))?
    }

    Ok(())
}

async fn poll(
    p: &Peripheral,
    device: &DeviceInfo,
    service_filter: &HashSet<Uuid>,
    characteristic_filter: &HashSet<Uuid>,
    interval: f64,
    json: bool,
    decode_map: &HashMap<Uuid, CharFormat>,
) -> anyhow::Result<()> {
    match timeout(Duration::from_secs(CONNECT_TIMEOUT), p.connect()).await {
        Ok(Ok(_)) => {
            match timeout(Duration::from_secs(ENUMERATE_TIMEOUT), p.discover_services()).await {
                Ok(Ok(_)) => {
                    let services = if service_filter.is_empty() {
                        p.services()
                    } else {
                        p.services()
                            .into_iter()
                            .filter(|s| service_filter.contains(&s.uuid))
                            .collect::<BTreeSet<_>>()
                    };
                    if !services.is_empty() {
                        let mut ticker = tokio::time::interval(Duration::from_millis((interval * 1000.0) as u64));
                        loop {
                            ticker.tick().await; // First tick returns immediately
                            let mut service_info = Vec::new();
                            for service in &services {
                                let mut chars = Vec::new();
                                for characteristic in &service.characteristics {
                                    if characteristic_filter.is_empty()
                                        || characteristic_filter.contains(&characteristic.uuid)
                                    {
                                        // Get raw and decoded values
                                        let (value, decoded) =
                                            if characteristic.properties.contains(CharPropFlags::READ) {
                                                let value = p.read(characteristic).await.ok();
                                                let decoded = value.as_ref().and_then(|v| {
                                                    decode_map.get(&characteristic.uuid).map(|fmt| fmt.decode(v))
                                                });
                                                (value, decoded)
                                            } else {
                                                (None, None)
                                            };
                                        chars.push(CharacteristicInfo {
                                            uuid: characteristic.uuid.to_short_string(),
                                            properties: format_properties(characteristic.properties),
                                            char_type: CHARACTERISTIC_MAP.get(&characteristic.uuid).map(|v| &**v),
                                            value,
                                            decoded,
                                        });
                                    }
                                }
                                // Dont add services with no matching characteristics
                                if !chars.is_empty() {
                                    service_info.push(ServiceInfo {
                                        uuid: service.uuid.to_short_string(),
                                        service_type: SERVICE_MAP.get(&service.uuid).map(|v| &**v),
                                        characteristics: chars,
                                    });
                                }
                            }
                            // Only show devices with matching services
                            if service_info.is_empty() {
                                break;
                            } else {
                                // Update rssi
                                let rssi = if let Ok(Some(properties)) = p.properties().await {
                                    properties.rssi.unwrap_or(0)
                                } else {
                                    0
                                };
                                let device = DeviceInfo {
                                    id: device.id.clone(),
                                    name: device.name.clone(),
                                    rssi,
                                    services: service_info.clone(),
                                };
                                if json {
                                    println!("{}", serde_json::to_string(&device)?);
                                } else {
                                    print!("[+] Device: {}", device);
                                }
                            }
                        }
                    }
                }
                _ => {
                    // eprintln!("Service discovery failed/timeout for {}", p.id()),
                }
            }
        }
        _ => {
            // eprintln!("Connect timeout/error for {}", p.id()),
        }
    }
    Ok(())
}
