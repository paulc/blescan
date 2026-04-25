use anyhow::Context;
use btleplug::platform::Adapter;
use regex::Regex;
use serde::Serialize;
use serde_json::json;
use tokio::sync::mpsc;
use uuid::Uuid;

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::commands::WriteArgs;
use crate::scanner::DeviceScanner;
use crate::util::{parse_uuid, parse_write, run_with_timeout, uuid_filter};

#[derive(Debug, Clone, Serialize)]
struct WriteStatus {
    uuid: Uuid,
    status: bool,
    error: Option<String>,
}

pub async fn run(central: Adapter, args: WriteArgs) -> anyhow::Result<()> {
    let device_filter = args
        .device
        .iter()
        .map(|s| parse_uuid(s))
        .collect::<Result<Vec<_>, _>>()
        .context("Error Parsing Device UUID")?;
    let name_filter = args
        .name
        .iter()
        .map(|s| Regex::new(s))
        .collect::<Result<Vec<_>, _>>()
        .context("Error parsing name regex")?;
    let service_filter = uuid_filter(&args.service)?;
    let write_map = parse_write(&args.characteristic)?;
    let characteristic_filter = Arc::new(write_map.keys().cloned().collect::<HashSet<Uuid>>());

    let n_write = Arc::new(AtomicU32::new(write_map.len() as u32));
    let (tx, mut rx) = mpsc::channel(1);
    let scan = async {
        let json = args.json;
        let mut scanner = DeviceScanner::start(central, args.rssi, name_filter, device_filter).await?;
        loop {
            tokio::select! {
                _ = rx.recv() => {
                    // max devices
                    break;
                }
                Ok(Some((peripheral, mut device))) = scanner.next_match() => {
                    tokio::spawn({
                        let service_filter = Arc::clone(&service_filter);
                        let characteristic_filter = Arc::clone(&characteristic_filter);
                        let write_map = Arc::clone(&write_map);
                        let n_write = Arc::clone(&n_write);
                        let tx = tx.clone();
                        async move {
                            let result = {
                                device.connect(&peripheral).await?;
                                device
                                    .enumerate(&peripheral, &service_filter, &characteristic_filter)
                                    .await?;
                                for service in device.services.values_mut() {
                                    for (uuid,characteristic) in &mut service.characteristics {
                                        let status = match characteristic.write(
                                            &peripheral,
                                            args.without_response,
                                            write_map.get(uuid).context(format!("Write data not found: {}", uuid))?
                                        ).await {
                                            Ok(_) => WriteStatus { uuid: *uuid, status: true, error: None },
                                            Err(e) => WriteStatus { uuid: *uuid, status: false, error: Some(e.to_string()) }
                                        };
                                        if json {
                                            println!("{}", serde_json::to_string(
                                                    &json!({ "write_status": status }))?);
                                        } else {
                                            print!("[+] Write Successful: {}", status.uuid)
                                        }
                                    if n_write.fetch_sub(1, Ordering::Relaxed) == 1 {
                                        // Previous value = 1 - signal completion
                                        let _ = tx.send(()).await;
                                    }
                                    }
                                }
                                device.disconnect(&peripheral).await?;
                                Ok::<(), anyhow::Error>(())
                            };
                            if let Err(e) = result {
                                eprintln!("Write Error: {}", e)
                            }
                            Ok::<(), anyhow::Error>(())
                        }
                    });
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    };

    run_with_timeout(args.timeout, args.json, scan).await
}
