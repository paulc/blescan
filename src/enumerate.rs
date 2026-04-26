use anyhow::Context;
use btleplug::platform::Adapter;
use regex::Regex;
use tokio::sync::Semaphore;
use tokio::sync::mpsc;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::MAX_TASKS;
use crate::commands::EnumerateArgs;
use crate::scanner::DeviceScanner;
use crate::util::{parse_decoder, parse_uuid, read_all_lines, run_with_timeout, uuid_filter};

pub async fn run(central: Adapter, args: EnumerateArgs) -> anyhow::Result<()> {
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
    let characteristic_filter = uuid_filter(&args.characteristic)?;
    let decode_map = parse_decoder(
        &args
            .decode
            .into_iter()
            .chain(read_all_lines(&args.decode_file)?) // Read from decode_files
            .collect::<Vec<_>>(),
    )?;
    let max = if let Some(max) = args.max {
        Arc::new(AtomicU32::new(max))
    } else {
        Arc::new(AtomicU32::new(u32::MAX))
    };
    let (tx, mut rx) = mpsc::channel(1);
    let task_semaphore = Arc::new(Semaphore::new(MAX_TASKS.load(Ordering::Relaxed)));

    let scan = async {
        let json = args.json;
        let read = args.read;
        let mut scanner = DeviceScanner::start(central, args.rssi, name_filter, device_filter).await?;
        loop {
            tokio::select! {
                _ = rx.recv() => {
                    // max devices
                    break;
                }
                s = scanner.next_match() => {
                    match s {
                        Ok(Some((peripheral, mut device))) => {
                            tokio::spawn({
                                let task_semaphore = Arc::clone(&task_semaphore);
                                let service_filter = Arc::clone(&service_filter);
                                let characteristic_filter = Arc::clone(&characteristic_filter);
                                let decode_map = Arc::clone(&decode_map);
                                let max = Arc::clone(&max);
                                let tx = tx.clone();
                                async move {
                                    // Limit running tasks using semaphore
                                    let _permit = task_semaphore.acquire().await?;
                                    let result = {
                                        device.connect(&peripheral).await?;
                                        device
                                            .enumerate(&peripheral, &service_filter, &characteristic_filter)
                                            .await?;
                                        if read {
                                            device.read(&peripheral, &decode_map).await?;
                                        }
                                        // If we have filters active only show matching devices
                                        if (service_filter.is_empty() && characteristic_filter.is_empty())
                                            || !device.services.is_empty()
                                        {
                                            if json {
                                                println!("{}", serde_json::to_string(&device)?)
                                            } else {
                                                print!("[+] Device: {}", device)
                                            }
                                            if max.fetch_sub(1, Ordering::Relaxed) == 1 {
                                                // Previous value = 1 - signal completion
                                                let _ = tx.send(()).await;
                                            }
                                        }
                                        device.disconnect(&peripheral).await?;
                                        Ok::<(), anyhow::Error>(())
                                    };
                                    if let Err(e) = result {
                                        eprintln!("Error: {e}")
                                    }
                                    Ok::<(), anyhow::Error>(())
                                }
                            });
                        },
                        Ok(None) => {
                            eprintln!("Error: BLE Event Stream Ended");
                            break;
                        }
                        Err(e) => {
                            eprintln!("Error: {}", e);
                            break;
                        }
                    }
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    };

    run_with_timeout(args.timeout, args.json, scan).await
}
