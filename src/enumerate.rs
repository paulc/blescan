use btleplug::platform::Adapter;
use tokio::sync::mpsc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use crate::commands::EnumerateArgs;
use crate::scanner::DeviceScanner;
use crate::util::{make_decode_map, make_regex_filter, make_uuid_filter, run_with_timeout};
use crate::MAX_TASKS;

pub async fn run(central: Adapter, args: EnumerateArgs) -> anyhow::Result<()> {
    let device_filter = args.device;
    let name_filter = make_regex_filter(&args.name)?;
    let service_filter = make_uuid_filter(&args.service)?;
    let characteristic_filter = make_uuid_filter(&args.characteristic)?;
    let decode_map = make_decode_map(&args.decode, &args.decode_file)?;

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
        let mut scanner = DeviceScanner::start(central, args.rssi, name_filter, device_filter, true).await?;
        let mut join_set: JoinSet<anyhow::Result<()>> = JoinSet::new();

        loop {
            tokio::select! {
                _ = rx.recv() => {
                    // max devices
                    break;
                }
                s = scanner.next_match() => {
                    match s {
                        Ok(Some((peripheral, mut device))) => {
                                let task_semaphore = Arc::clone(&task_semaphore);
                                let service_filter = Arc::clone(&service_filter);
                                let characteristic_filter = Arc::clone(&characteristic_filter);
                                let decode_map = Arc::clone(&decode_map);
                                let max = Arc::clone(&max);
                                let tx = tx.clone();
                            join_set.spawn(
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
                                    Ok(())
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

        while let Some(result) = join_set.join_next().await {
            result??; // JoinError (panic/cancel) / anyhow::Error
        }

        Ok::<(), anyhow::Error>(())
    };

    run_with_timeout(args.timeout, args.json, scan).await
}
