use anyhow::Context;
use btleplug::api::Peripheral as _;
use btleplug::platform::Adapter;
use futures::StreamExt;
use regex::Regex;
use serde_json::json;

use std::sync::Arc;

use crate::commands::NotifyArgs;
use crate::scanner::DeviceScanner;
use crate::types::NotificationData;
use crate::util::{parse_decoder, parse_uuid, read_all_lines, run_with_timeout, uuid_filter};

pub async fn run(central: Adapter, args: NotifyArgs) -> anyhow::Result<()> {
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
    let scan = async {
        let json = args.json;
        let mut scanner = DeviceScanner::start(central, args.rssi, name_filter, device_filter).await?;
        while let Some((peripheral, mut device)) = scanner.next_match().await? {
            tokio::spawn({
                let service_filter = Arc::clone(&service_filter);
                let characteristic_filter = Arc::clone(&characteristic_filter);
                let decode_map = Arc::clone(&decode_map);
                async move {
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
                                .and_then(|fmt| fmt.decode_value(&notification.value).ok());
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
                }
            });
        }
        Ok::<(), anyhow::Error>(())
    };

    run_with_timeout(args.timeout, args.json, scan).await
}
