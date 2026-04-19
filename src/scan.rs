use anyhow::anyhow;
use btleplug::api::{Central, CentralEvent, ScanFilter};
use btleplug::platform::Adapter;
use futures::StreamExt;
use regex::Regex;
use std::time::Duration;
use tokio::time::timeout;

use std::collections::HashSet;

use crate::commands::ScanArgs;
use crate::filter::device_match;
use crate::types::DeviceInfo;

pub async fn run(central: Adapter, args: ScanArgs) -> anyhow::Result<()> {
    let name_filter = args.name.iter().map(|s| Regex::new(s)).collect::<Result<Vec<_>, _>>()?;

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

                            if args.json {
                                println!("{}", serde_json::to_string(&device)?);
                            } else {
                                print!("[+] Device: {}", device);
                            }
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
