use anyhow::Context;
use btleplug::platform::Adapter;
use regex::Regex;

use crate::commands::ScanArgs;
use crate::scanner::DeviceScanner;
use crate::util::{parse_uuid, run_with_timeout};

pub async fn run(central: Adapter, args: ScanArgs) -> anyhow::Result<()> {
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
    let scan = async {
        let mut scanner = DeviceScanner::start(central, args.rssi, name_filter, device_filter).await?;
        while let Some((_peripheral, device)) = scanner.next_match().await? {
            if args.json {
                println!("{}", serde_json::to_string(&device)?);
            } else {
                print!("[+] Device: {}", device);
            }
        }
        Ok::<(), anyhow::Error>(())
    };
    run_with_timeout(args.timeout, args.json, scan).await
}
