use anyhow::{anyhow, Context};
use btleplug::api::{Central, CentralEvent, ScanFilter};
use btleplug::platform::Adapter;
use futures::StreamExt;
use std::time::Duration;
use tokio::time::timeout;
use uuid::Uuid;

use std::collections::{HashMap, HashSet};

use crate::device::{device_info, enumerate_services};
use crate::device_info::DeviceInfo;
use crate::util::{hex_to_vec, parse_uuid};
use crate::WriteArgs;

pub async fn run(central: Adapter, args: WriteArgs) -> anyhow::Result<()> {
    if args.service.is_empty() || args.characteristic.is_empty() {
        anyhow::bail!("write requires --service and --characteristic");
    }

    // Convert service filter to HashSet<Uuid>
    let service_filter = args
        .service
        .iter()
        .map(|s| parse_uuid(s))
        .collect::<Result<HashSet<Uuid>, _>>()
        .context("Error Parsing Service UUID")?;

    // Convert characteristic filter to HashMap<Uuid,Vec<u8>>
    let characteristic_filter = args
        .characteristic
        .iter()
        .map(|s| {
            s.split_once("::")
                .with_context(|| "Invalid data format: <uuid::data (hex)>")
                .and_then(|(uuid_str, data_str)| {
                    let uuid = parse_uuid(uuid_str)?;
                    let data = hex_to_vec(data_str)?;
                    Ok((uuid, data))
                })
        })
        .collect::<Result<HashMap<_, _>, _>>()?;

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
                            let device = device_info(&peripheral).await?;
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
                            let service_filter = service_filter.clone();
                            let characteristic_filter = characteristic_filter.clone();

                            tokio::spawn(async move {});
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
