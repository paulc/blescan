use btleplug::platform::Adapter;
use tokio::sync::Semaphore;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::MAX_TASKS;
use crate::commands::PollArgs;
use crate::scanner::DeviceScanner;
use crate::util::{make_decode_map, make_regex_filter, make_uuid_filter, run_with_timeout};

pub async fn run(central: Adapter, args: PollArgs) -> anyhow::Result<()> {
    let device_filter = args.device;
    let name_filter = make_regex_filter(&args.name)?;
    let service_filter = make_uuid_filter(&args.service)?;
    let characteristic_filter = make_uuid_filter(&args.characteristic)?;
    let decode_map = make_decode_map(&args.decode, &args.decode_file)?;
    let task_semaphore = Arc::new(Semaphore::new(MAX_TASKS.load(Ordering::Relaxed)));

    let scan = async {
        let json = args.json;
        let interval = args.interval;
        let mut scanner = DeviceScanner::start(central, args.rssi, name_filter, device_filter, true).await?;
        while let Some((peripheral, mut device)) = scanner.next_match().await? {
            tokio::spawn({
                let task_semaphore = Arc::clone(&task_semaphore);
                let service_filter = Arc::clone(&service_filter);
                let characteristic_filter = Arc::clone(&characteristic_filter);
                let decode_map = Arc::clone(&decode_map);
                async move {
                    // Limit running tasks using semaphore
                    let _permit = task_semaphore.acquire().await?;
                    device.connect(&peripheral).await?;
                    device
                        .enumerate(&peripheral, &service_filter, &characteristic_filter)
                        .await?;
                    let mut ticker = tokio::time::interval(Duration::from_millis((interval * 1000.0) as u64));
                    if (service_filter.is_empty() && characteristic_filter.is_empty()) || !device.services.is_empty() {
                        let e = {
                            // First tick returns immediately
                            loop {
                                ticker.tick().await;
                                if let Err(e) = device.read(&peripheral, &decode_map).await {
                                    break e;
                                };
                                if json {
                                    println!("{}", serde_json::to_string(&device).unwrap())
                                } else {
                                    print!("[+] Device: {}", device)
                                }
                            }
                        };
                        Err::<(), anyhow::Error>(e)
                    } else {
                        Ok::<(), anyhow::Error>(())
                    }
                }
            });
        }
        Ok::<(), anyhow::Error>(())
    };

    run_with_timeout(args.timeout, args.json, scan).await
}
