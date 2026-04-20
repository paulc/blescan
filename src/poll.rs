use anyhow::{Context, anyhow};
use btleplug::api::{Central, CentralEvent, Characteristic, ScanFilter, Service};
use btleplug::platform::Adapter;
use btleplug::platform::Peripheral;
use futures::StreamExt;
use regex::Regex;
use tokio::sync::Mutex;
use tokio::time::timeout;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use crate::commands::PollArgs;
use crate::filter::{device_match, filter};
use crate::types::{CharacteristicInfo, DeviceInfo, ServiceInfo};
use crate::util::{parse_decoder, parse_uuid, read_all_lines, uuid_filter};

pub async fn run(central: Adapter, args: PollArgs) -> anyhow::Result<()> {
    let service_filter = uuid_filter(&args.service)?;
    let characteristic_filter = uuid_filter(&args.characteristic)?;
    let name_filter = args.name.iter().map(|s| Regex::new(s)).collect::<Result<Vec<_>, _>>()?;
    let decode_map = parse_decoder(
        &args
            .decode
            .into_iter()
            .chain(read_all_lines(&args.decode_file)?) // Read from decode_files
            .collect::<Vec<_>>(),
    )?;

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
                            //
                            if !device_match(&device, &args.rssi, &name_filter, &args.device) {
                                continue;
                            }

                            let device = Arc::new(Mutex::new(device));

                            // Standard match callback - add matching characteristics to devivce
                            let match_callback = {
                                let device = Arc::clone(&device);
                                async move |_peripheral: &Peripheral,
                                            service: &Service,
                                            characteristic: &Characteristic|
                                            -> anyhow::Result<()> {
                                    let mut device = device.lock().await;
                                    let c = CharacteristicInfo::new(&characteristic);
                                    device
                                        .services
                                        .entry(service.uuid.clone())
                                        .or_insert(ServiceInfo::new(&service))
                                        .characteristics
                                        .insert(characteristic.uuid.clone(), c);
                                    Ok(())
                                }
                            };

                            let completed_callback = {
                                let device = Arc::clone(&device);
                                let decode_map = Arc::clone(&decode_map);
                                async move |peripheral: &Peripheral| -> anyhow::Result<()> {
                                    let mut device = device.lock().await;
                                    if !device.services.is_empty() {
                                        let mut ticker = tokio::time::interval(Duration::from_millis(
                                            (args.interval * 1000.0) as u64,
                                        ));
                                        loop {
                                            // First tick returns immediately
                                            ticker.tick().await;
                                            // Update device data
                                            device.update_rssi(peripheral).await;
                                            for service in device.services.values_mut() {
                                                for characteristic in service.characteristics.values_mut() {
                                                    characteristic.read(peripheral, &decode_map).await;
                                                }
                                            }
                                            if args.json {
                                                println!("{}", serde_json::to_string(&*device)?);
                                            } else {
                                                print!("[+] Device: {}", device);
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
