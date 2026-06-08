use anyhow::Context;
use btleplug::api::bleuuid::BleUuid;
use btleplug::platform::Adapter;
use serde::Serialize;
use serde_json::json;
use tokio::sync::mpsc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio::time::{sleep, Duration};

use uuid::Uuid;

use std::collections::HashSet;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use crate::commands::WriteReadArgs;
use crate::scanner::DeviceScanner;
use crate::util::{make_decode_map, make_regex_filter, make_uuid_filter, parse_write, run_with_timeout, serialize_hex};
use crate::MAX_TASKS;

#[derive(Debug, Clone, Serialize)]
struct WriteStatus<'a> {
    device: &'a str,
    service: &'a Uuid,
    characteristic: &'a Uuid,
    #[serde(serialize_with = "serialize_hex")]
    data: &'a Vec<u8>,
    status: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl<'a> std::fmt::Display for WriteStatus<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(ref error) = self.error {
            write!(
                f,
                "[-] Write Error: Device: {}\n                   └─ Service: {}\n                      └─ Characteristic: {} [{}]",
                self.device,
                self.service.to_short_string(),
                self.characteristic.to_short_string(),
                error
            )
        } else {
            write!(
                f,
                "[+] Write Successful: Device: {}\n                      └─ Service: {}\n                         └─ Characteristic: {} [{}]",
                self.device,
                self.service.to_short_string(),
                self.characteristic.to_short_string(),
                hex::encode(self.data)
            )
        }
    }
}

pub async fn run(central: Adapter, args: WriteReadArgs) -> anyhow::Result<()> {
    let device_filter = args.device;
    let name_filter = make_regex_filter(&args.name)?;
    let service_filter = make_uuid_filter(&args.service)?;
    let write_map = parse_write(&args.characteristic)?;
    let characteristic_filter = Arc::new(write_map.keys().cloned().collect::<HashSet<Uuid>>());
    let decode_map = make_decode_map(&args.decode, &args.decode_file)?;

    // Numer of characteristics to write - uses I32 to avoid wrapping
    // Assumes only matches a single device
    let n_write = Arc::new(AtomicI32::new(write_map.len() as i32));
    let (tx, mut rx) = mpsc::channel(1);
    let task_semaphore = Arc::new(Semaphore::new(MAX_TASKS.load(Ordering::Relaxed)));

    let scan = async {
        let json = args.json;
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
                            let write_map = Arc::clone(&write_map);
                            let decode_map = Arc::clone(&decode_map);
                            let n_write = Arc::clone(&n_write);
                            let tx = tx.clone();
                            join_set.spawn(async move {
                                // Limit running tasks using semaphore
                                let _permit = task_semaphore.acquire().await?;
                                let result = async {
                                    device.connect(&peripheral).await?;
                                    device
                                        .enumerate(&peripheral, &service_filter, &characteristic_filter)
                                        .await?;
                                    let mut device_read = device.clone();
                                    for service in device.services.values_mut() {
                                        for (uuid,characteristic) in &mut service.characteristics {
                                            let data = write_map.get(uuid).context(format!("Write data not found: {}", uuid))?;
                                            let status = match characteristic.write(
                                                    &peripheral,
                                                    args.without_response,
                                                    data
                                                ).await {
                                                    Ok(_) => WriteStatus {
                                                        device: &device.id,
                                                        service: &service.uuid,
                                                        characteristic: uuid,
                                                        data,
                                                        status: true,
                                                        error: None
                                                    },
                                                    Err(e) => WriteStatus {
                                                        device: &device.id,
                                                        service: &service.uuid,
                                                        characteristic: uuid,
                                                        data,
                                                        status: false,
                                                        error: Some(e.to_string())
                                                    }
                                                };
                                            if json {
                                                println!("{}", serde_json::to_string(
                                                        &json!({ "write_status": status }))?);
                                            } else {
                                                println!("{}", status);
                                            }
                                            // Read device data
                                            if let Some(delay_ms) = args.read_delay {
                                                sleep(Duration::from_millis(delay_ms)).await;
                                            }
                                            device_read.read(&peripheral, &decode_map).await?;
                                            if json {
                                                println!("{}", serde_json::to_string(&device_read)?)
                                            } else {
                                                print!("[+] Device: {}", device_read)
                                            }
                                            if n_write.fetch_sub(1, Ordering::Relaxed) == 1 {
                                                // Exit when previous value == 1
                                                // Note: If the write characteristics match multiple
                                                // devices this will exit after n_write which may
                                                // not be correct - filter should ensure that there
                                                // is only one matching device
                                                let _ = tx.send(()).await;
                                            }
                                        }
                                    }
                                    Ok::<(), anyhow::Error>(())
                                }.await;
                                let _ = device.disconnect(&peripheral).await;
                                if let Err(_e) = result {
                                    // Ignore connect/timeout errors (likely to be other devices timing out)
                                    // eprintln!("Write Error: {}", e)
                                }
                                Ok::<(), anyhow::Error>(())
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
