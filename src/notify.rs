use anyhow::{Context, anyhow};
use btleplug::api::{Central, CentralEvent, CharPropFlags, Peripheral as _, ScanFilter, bleuuid::BleUuid};
use btleplug::platform::{Adapter, Peripheral};
use futures::StreamExt;
use std::time::Duration;
use tokio::time::timeout;
use uuid::Uuid;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use crate::NotifyArgs;
use crate::char_data::CharFormat;
use crate::types::{DeviceInfo, NotificationInfo};
use crate::util::{parse_decoder, parse_uuid, uuid_filter};
use crate::{CONNECT_TIMEOUT, DISCONNECT_TIMEOUT, ENUMERATE_TIMEOUT};

pub async fn run(central: Adapter, args: NotifyArgs) -> anyhow::Result<()> {
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

                            // Connect to device in background
                            let service_filter = Arc::clone(&service_filter);
                            let characteristic_filter = Arc::clone(&characteristic_filter);
                            let decode_map = Arc::clone(&decode_map);

                            tokio::spawn(async move {
                                notify(
                                    &peripheral,
                                    &service_filter,
                                    &characteristic_filter,
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

async fn notify(
    peripheral: &Peripheral,
    service_filter: &HashSet<Uuid>,
    characteristic_filter: &HashSet<Uuid>,
    json: bool,
    decode_map: &HashMap<Uuid, CharFormat>,
) -> anyhow::Result<()> {
    match timeout(Duration::from_secs(CONNECT_TIMEOUT), peripheral.connect()).await {
        Ok(Ok(_)) => {
            match timeout(Duration::from_secs(ENUMERATE_TIMEOUT), peripheral.discover_services()).await {
                Ok(Ok(_)) => {
                    let services = if service_filter.is_empty() {
                        peripheral.services()
                    } else {
                        peripheral
                            .services()
                            .into_iter()
                            .filter(|s| service_filter.contains(&s.uuid))
                            .collect::<BTreeSet<_>>()
                    };
                    let mut subscribed = false;
                    for service in &services {
                        for characteristic in &service.characteristics {
                            if characteristic_filter.is_empty() || characteristic_filter.contains(&characteristic.uuid)
                            {
                                // Subscribe to matching characteristics
                                if characteristic.properties.contains(CharPropFlags::NOTIFY) {
                                    peripheral.subscribe(&characteristic).await?;
                                    if !json {
                                        eprintln!(
                                            "Subscribed :: Device: {}\n              └─ Service: {}\n                  └─ Characteristic: {}",
                                            peripheral.id(),
                                            service.uuid,
                                            characteristic.uuid
                                        );
                                    }
                                    subscribed = true;
                                }
                            }
                        }
                    }
                    // Listen for notifications if we have subscribed to any characteristics
                    if subscribed {
                        let mut notification_stream = peripheral.notifications().await?;
                        while let Some(notification) = notification_stream.next().await {
                            let decoded = decode_map
                                .get(&notification.uuid)
                                .map(|fmt| fmt.decode(&notification.value));
                            let n = NotificationInfo {
                                service: notification.service_uuid.to_short_string(),
                                characteristic: notification.uuid.to_short_string(),
                                value: notification.value,
                                decoded,
                            };
                            if json {
                                println!("{}", serde_json::to_string(&n)?);
                            } else {
                                println!("{}", n);
                            }
                        }
                    }
                    if let Err(_e) = timeout(Duration::from_secs(DISCONNECT_TIMEOUT), peripheral.disconnect()).await {
                        // eprintln!("Disconnect timeout/error for {}: {:?}", p.id(), e);
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
