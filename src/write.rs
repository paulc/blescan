use anyhow::{Context, anyhow};
use btleplug::api::{
    Central, CentralEvent, CharPropFlags, Characteristic, Peripheral as _, ScanFilter, Service, WriteType,
};
use btleplug::platform::Adapter;
use btleplug::platform::Peripheral;
use futures::StreamExt;
use regex::Regex;
use tokio::sync::mpsc;
use tokio::time::timeout;
use uuid::Uuid;

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use crate::WRITE_TIMEOUT;
use crate::commands::WriteArgs;
use crate::filter::{device_match, filter};
use crate::types::DeviceInfo;
use crate::util::{parse_uuid, parse_write, uuid_filter};

pub async fn run(central: Adapter, args: WriteArgs) -> anyhow::Result<()> {
    let service_filter = uuid_filter(&args.service)?;
    let write_map = parse_write(&args.characteristic)?;
    let characteristic_filter = Arc::new(write_map.keys().cloned().collect::<HashSet<Uuid>>());
    let name_filter = args.name.iter().map(|s| Regex::new(s)).collect::<Result<Vec<_>, _>>()?;
    let n_write = Arc::new(AtomicU32::new(write_map.len() as u32));
    let (tx, mut rx) = mpsc::channel(1);

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
        // while let Some(event) = events.next().await {
        loop {
            tokio::select! {
                    _ = rx.recv() => {
                             // writes completed
                             break;
                        }
                    Some(event) = events.next() => {
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

                                            let match_callback = {
                                                let write_map = Arc::clone(&write_map);
                                                let n_write = Arc::clone(&n_write);
                                                let tx = tx.clone();
                                                async move |peripheral: &Peripheral,
                                                            _service: &Service,
                                                            characteristic: &Characteristic|
                                                            -> anyhow::Result<()> {
                                                    if characteristic.properties.contains(CharPropFlags::WRITE)
                                                        || characteristic
                                                            .properties
                                                            .contains(CharPropFlags::WRITE_WITHOUT_RESPONSE)
                                                    {
                                                        match timeout(
                                                            Duration::from_secs(WRITE_TIMEOUT),
                                                            peripheral.write(
                                                                characteristic,
                                                                write_map.get(&characteristic.uuid).context(format!(
                                                                    "Write data not found: {}",
                                                                    characteristic.uuid
                                                                ))?,
                                                                if characteristic
                                                                    .properties
                                                                    .contains(CharPropFlags::WRITE_WITHOUT_RESPONSE)
                                                                    || args.without_response
                                                                {
                                                                    WriteType::WithoutResponse
                                                                } else {
                                                                    WriteType::WithResponse
                                                                },
                                                            ),
                                                        )
                                                        .await
                                                        {
                                                            Ok(Ok(_)) => eprintln!("Write Successful: {}", characteristic.uuid),
                                                            Ok(Err(e)) => eprintln!("Write Error: {} -> {}", characteristic.uuid, e),
                                                            Err(_) => eprintln!("Write Timeout: {}", characteristic.uuid),
                                                        }
                                                        if n_write.fetch_sub(1, Ordering::Relaxed) == 1 {
                                                            // Previous value = 1 - signal completion
                                                            let _ = tx.send(()).await;
                                                        }
                                                    }
                                                    Ok(())
                                                }
                                            };

                                            let completed_callback =
                                                { async move |_peripheral: &Peripheral| -> anyhow::Result<()> { Ok(()) } };

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
