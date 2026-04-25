use anyhow::Context;
use btleplug::platform::Adapter;
use regex::Regex;

use std::sync::Arc;
use std::time::Duration;

use crate::commands::PollArgs;
use crate::scanner::DeviceScanner;
use crate::util::{parse_decoder, parse_uuid, read_all_lines, run_with_timeout, uuid_filter};

pub async fn run(central: Adapter, args: PollArgs) -> anyhow::Result<()> {
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
        let interval = args.interval;
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
                    let mut ticker = tokio::time::interval(Duration::from_millis((interval * 1000.0) as u64));
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
                    Err::<(), anyhow::Error>(e.into())
                }
            });
        }
        Ok::<(), anyhow::Error>(())
    };

    run_with_timeout(args.timeout, args.json, scan).await
}
