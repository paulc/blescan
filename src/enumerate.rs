use anyhow::{Context, anyhow};
use btleplug::api::{Central, CentralEvent, ScanFilter};
use btleplug::api::{CharPropFlags, Peripheral as _, bleuuid::BleUuid};
use btleplug::platform::Adapter;
use btleplug::platform::Peripheral;
use futures::StreamExt;
use std::time::Duration;
use tokio::time::timeout;
use uuid::Uuid;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::EnumerateArgs;
use crate::char_data::CharFormat;
use crate::types::{CharacteristicInfo, DeviceInfo, ServiceInfo};
use crate::util::{format_properties, parse_decoder, parse_uuid, uuid_filter};
use crate::{CHARACTERISTIC_MAP, SERVICE_MAP};
use crate::{CONNECT_TIMEOUT, DISCONNECT_TIMEOUT, ENUMERATE_TIMEOUT};

static MATCHES: AtomicU32 = AtomicU32::new(0);

pub async fn run(central: Adapter, args: EnumerateArgs) -> anyhow::Result<()> {
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
                            // Spawn enumeration in background so we don't block events
                            let service_filter = Arc::clone(&service_filter);
                            let characteristic_filter = Arc::clone(&characteristic_filter);
                            let decode_map = Arc::clone(&decode_map);

                            tokio::spawn(async move {
                                match enumerate_services(
                                    &peripheral,
                                    args.read,
                                    &service_filter,
                                    &characteristic_filter,
                                    &decode_map,
                                )
                                .await
                                {
                                    Ok(Some(services)) => {
                                        // Update DeviceInfo with discovered services
                                        let device = DeviceInfo { services, ..device };
                                        if args.json {
                                            println!("{}", serde_json::to_string(&device)?);
                                        } else {
                                            print!("[+] Discovered: {}", device);
                                        }
                                        MATCHES.fetch_add(1, Ordering::Relaxed);
                                    }
                                    Ok(None) => {
                                        // No filter matches
                                    }
                                    Err(e) => eprintln!("Enumeration error: {:?}", e),
                                }
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
            if args.max.is_some_and(|max| MATCHES.load(Ordering::Relaxed) >= max) {
                break;
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

// Distinguish between no services (empty Some<Vec>) and no filter matches (None)
async fn enumerate_services(
    p: &Peripheral,
    read: bool,
    service_filter: &HashSet<Uuid>,
    characteristic_filter: &HashSet<Uuid>,
    decode_map: &HashMap<Uuid, CharFormat>,
) -> anyhow::Result<Option<Vec<ServiceInfo>>> {
    let mut service_info = Vec::new();
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
                    for service in services {
                        let mut chars = Vec::new();
                        for characteristic in &service.characteristics {
                            if characteristic_filter.is_empty() || characteristic_filter.contains(&characteristic.uuid)
                            {
                                // Get raw and decoded values
                                let (value, decoded) =
                                    if read && characteristic.properties.contains(CharPropFlags::READ) {
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
                        // If characteristic filter is not empty only add services with
                        // matching characteristics
                        if characteristic_filter.is_empty() || !chars.is_empty() {
                            service_info.push(ServiceInfo {
                                uuid: service.uuid.to_short_string(),
                                service_type: SERVICE_MAP.get(&service.uuid).map(|v| &**v),
                                characteristics: chars,
                            });
                        }
                    }
                }
                _ => {
                    // eprintln!("Service discovery failed/timeout for {}", p.id())
                }
            }
            if let Err(_e) = timeout(Duration::from_secs(DISCONNECT_TIMEOUT), p.disconnect()).await {
                // eprintln!("Disconnect timeout/error for {}: {:?}", p.id(), e);
            }
        }
        _ => {
            // eprintln!("Connect timeout/error for {}", p.id())
        }
    }
    // If filters are active return None if no service/characteristic matches
    if service_info.is_empty() && (!service_filter.is_empty() || !characteristic_filter.is_empty()) {
        Ok(None)
    } else {
        Ok(Some(service_info))
    }
}
