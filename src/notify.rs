use btleplug::api::Peripheral as _;
use btleplug::platform::Adapter;
use futures::StreamExt;
use serde_json::json;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::commands::NotifyArgs;
use crate::scanner::DeviceScanner;
use crate::types::NotificationData;
use crate::util::{make_decode_map, make_regex_filter, make_uuid_filter, run_with_timeout};
use crate::MAX_TASKS;

pub async fn run(central: Adapter, args: NotifyArgs) -> anyhow::Result<()> {
    let device_filter = args.device;
    let name_filter = make_regex_filter(&args.name)?;
    let service_filter = make_uuid_filter(&args.service)?;
    let characteristic_filter = make_uuid_filter(&args.characteristic)?;
    let decode_map = make_decode_map(&args.decode, &args.decode_file)?;
    let task_semaphore = Arc::new(Semaphore::new(MAX_TASKS.load(Ordering::Relaxed)));

    let scan = async {
        let json = args.json;
        let mut scanner = DeviceScanner::start(central, args.rssi, name_filter, device_filter, true).await?;
        let mut join_set: JoinSet<anyhow::Result<()>> = JoinSet::new();

        while let Some((peripheral, mut device)) = scanner.next_match().await? {
            let task_semaphore = Arc::clone(&task_semaphore);
            let service_filter = Arc::clone(&service_filter);
            let characteristic_filter = Arc::clone(&characteristic_filter);
            let decode_map = Arc::clone(&decode_map);
            join_set.spawn(async move {
                // Limit running tasks using semaphore
                let _permit = task_semaphore.acquire().await?;
                let result = async {
                    device.connect(&peripheral).await?;
                    device
                        .enumerate(&peripheral, &service_filter, &characteristic_filter)
                        .await?;
                    let subscriptions = device.subscribe(&peripheral).await?;
                    for s in &subscriptions {
                        if json {
                            println!("{}", json!({ "subscription": s }));
                        } else {
                            println!("{}", s)
                        }
                    }
                    if !subscriptions.is_empty() {
                        let mut notification_stream = peripheral.notifications().await?;
                        while let Some(notification) = notification_stream.next().await {
                            let decoded = decode_map
                                .get(&notification.uuid)
                                .and_then(|fmt| fmt.decode(&notification.value).ok());
                            let n = NotificationData {
                                service: notification.service_uuid,
                                characteristic: notification.uuid,
                                value: notification.value,
                                decoded,
                            };
                            if json {
                                println!("{}", json!({ "notification" : n}));
                            } else {
                                println!("{}", n);
                            }
                        }
                    }
                    Ok::<(), anyhow::Error>(())
                }.await;
                // Always disconnect so the peripheral isn't left subscribed/
                // connected when the task ends (stream closed, timeout, or error).
                let _ = device.disconnect(&peripheral).await;
                result
            });
        }

        while let Some(result) = join_set.join_next().await {
            result??; // JoinError (panic/cancel) / anyhow::Error
        }

        Ok::<(), anyhow::Error>(())
    };

    run_with_timeout(args.timeout, args.json, scan).await
}
