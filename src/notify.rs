use anyhow::{Context, anyhow};
use btleplug::api::{Central, CentralEvent, CharPropFlags, Characteristic, Peripheral as _, ScanFilter, Service};
use btleplug::platform::Adapter;
use btleplug::platform::Peripheral;
use futures::StreamExt;
use regex::Regex;
use tokio::time::timeout;

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::commands::NotifyArgs;
use crate::filter::{device_match, filter};
use crate::types::{DeviceInfo, NotificationInfo};
use crate::util::{parse_decoder, parse_uuid, uuid_filter};

pub async fn run(central: Adapter, args: NotifyArgs) -> anyhow::Result<()> {
    let service_filter = uuid_filter(&args.service)?;
    let characteristic_filter = uuid_filter(&args.characteristic)?;
    let name_filter = args.name.iter().map(|s| Regex::new(s)).collect::<Result<Vec<_>, _>>()?;
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

                            if !device_match(&device, &args.rssi, &name_filter, &args.device) {
                                continue;
                            }

                            let subscribed = Arc::new(AtomicBool::new(false));

                            let match_callback = {
                                let json = args.json;
                                let subscribed = Arc::clone(&subscribed);
                                async move |peripheral: &Peripheral,
                                            service: &Service,
                                            characteristic: &Characteristic|
                                            -> anyhow::Result<()> {
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
                                        subscribed.store(true, Ordering::Relaxed);
                                    }
                                    Ok(())
                                }
                            };

                            let completed_callback = {
                                let decode_map = Arc::clone(&decode_map);
                                let json = args.json;
                                let subscribed = Arc::clone(&subscribed);
                                async move |peripheral: &Peripheral| -> anyhow::Result<()> {
                                    if subscribed.load(Ordering::Relaxed) {
                                        let mut notification_stream = peripheral.notifications().await?;
                                        while let Some(notification) = notification_stream.next().await {
                                            let decoded = decode_map
                                                .get(&notification.uuid)
                                                .map(|fmt| fmt.decode(&notification.value));
                                            let n = NotificationInfo {
                                                service: notification.service_uuid,
                                                characteristic: notification.uuid,
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
                                    Ok(())
                                }
                            };

                            // Spawn enumeration in background so we don't block events
                            let service_filter = Arc::clone(&service_filter);
                            let characteristic_filter = Arc::clone(&characteristic_filter);

                            tokio::spawn(async move {
                                filter(
                                    &peripheral,
                                    &service_filter,
                                    &characteristic_filter,
                                    match_callback,
                                    completed_callback,
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
