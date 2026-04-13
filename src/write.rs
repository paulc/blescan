use anyhow::{anyhow, Context};
use btleplug::api::{Central, CentralEvent, CharPropFlags, Peripheral as _, ScanFilter, WriteType};
use btleplug::platform::Adapter;
use futures::StreamExt;
use std::time::Duration;
use tokio::time::timeout;

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::char_data::CharData;
use crate::device_info::DeviceInfo;
use crate::util::parse_uuid;
use crate::WriteArgs;
use crate::{CONNECT_TIMEOUT, DISCONNECT_TIMEOUT, ENUMERATE_TIMEOUT, WRITE_TIMEOUT};

static WRITE_COMPLETE: AtomicBool = AtomicBool::new(false);

pub async fn run(central: Adapter, args: WriteArgs) -> anyhow::Result<()> {
    // Convert args
    let service_match = parse_uuid(&args.service).context("Error Parsing Service UUID")?;
    let characteristic_match =
        parse_uuid(&args.characteristic).context("Error Parsing Characteristic UUID")?;
    let data =
        CharData::try_from(args.data.as_str()).context("Error Parsing Characteristic Data")?;

    // Check args.device is a valid UUID
    if let Some(ref device) = args.device {
        parse_uuid(&device).context("Error Parsing Device UUID")?;
    }

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
                            if args.name.as_ref().is_some_and(|name| *name != device.name) {
                                continue;
                            }

                            // Filter by device UUID
                            if args.device.as_ref().is_some_and(|id| *id != device.id) {
                                continue;
                            }

                            // Spawn enumeration in background so we don't block events
                            let data = data.clone();
                            tokio::spawn(async move {
                                match timeout(
                                    Duration::from_secs(CONNECT_TIMEOUT),
                                    peripheral.connect(),
                                )
                                .await
                                {
                                    Ok(Ok(_)) => {
                                        match timeout(
                                            Duration::from_secs(ENUMERATE_TIMEOUT),
                                            peripheral.discover_services(),
                                        )
                                        .await
                                        {
                                            Ok(Ok(_)) => {
                                                'services: for service in peripheral.services() {
                                                    if service.uuid == service_match {
                                                        for char in &service.characteristics {
                                                            if char.uuid == characteristic_match {
                                                                if char
                                                                    .properties
                                                                    .contains(CharPropFlags::WRITE)
                                                                {
                                                                    match timeout(
                                                                        Duration::from_secs(WRITE_TIMEOUT),
                                                                        peripheral
                                                                        .write(
                                                                            char,
                                                                            data.to_vec(),
                                                                            WriteType::WithResponse,
                                                                        ))
                                                                        .await
                                                                    {
                                                                        Ok(Ok(_)) => eprintln!(
                                                                            "Write Successful: {}",
                                                                            char.uuid
                                                                        ),
                                                                        Ok(Err(e)) => eprintln!(
                                                                            "Write Error: {} -> {}",
                                                                            char.uuid, e
                                                                        ),
                                                                        Err(_) => eprintln!(
                                                                            "Write Timeout: {}",
                                                                            char.uuid
                                                                        ),
                                                                    }
                                                                } else if char
                                                                    .properties
                                                                    .contains(CharPropFlags::WRITE_WITHOUT_RESPONSE)
                                                                {
                                                                    match peripheral
                                                                        .write(
                                                                            char,
                                                                            data.to_vec(),
                                                                            WriteType::WithoutResponse,
                                                                        )
                                                                        .await
                                                                    {
                                                                        Ok(_) => eprintln!(
                                                                            "Write Successful: {}",
                                                                            char.uuid
                                                                        ),
                                                                        Err(e) => eprintln!(
                                                                            "Write Error: {} -> {}",
                                                                            char.uuid, e
                                                                        ),
                                                                    }
                                                                } else {
                                                                    eprintln!("ERROR: Characteristic {} not writeable", char.uuid);
                                                                }
                                                                WRITE_COMPLETE
                                                                    .store(true, Ordering::Relaxed);
                                                                break 'services;
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            _ => {
                                                eprintln!(
                                                    "Service discovery failed/timeout for {}",
                                                    peripheral.id()
                                                )
                                            }
                                        }
                                        if let Err(e) = timeout(
                                            Duration::from_secs(DISCONNECT_TIMEOUT),
                                            peripheral.disconnect(),
                                        )
                                        .await
                                        {
                                            eprintln!(
                                                "Disconnect timeout/error for {}: {:?}",
                                                peripheral.id(),
                                                e
                                            );
                                        }
                                    }
                                    _ => {
                                        eprintln!("Connect timeout/error for {}", peripheral.id())
                                    }
                                }
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
            if WRITE_COMPLETE.load(Ordering::Relaxed) {
                break;
            }
        }

        // Check that we wrote the data
        if WRITE_COMPLETE.load(Ordering::Relaxed) {
            Ok::<(), anyhow::Error>(())
        } else {
            anyhow::bail!("Error writing BLE data")
        }
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
