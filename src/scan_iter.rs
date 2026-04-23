use btleplug::platform::Adapter;
use regex::Regex;

use std::collections::{HashMap, HashSet};

use crate::commands::ScanIterArgs;
use crate::scanner::DeviceScanner;
use crate::util::run_with_timeout;

pub async fn run(central: Adapter, args: ScanIterArgs) -> anyhow::Result<()> {
    let name_filter = args.name.iter().map(|s| Regex::new(s)).collect::<Result<Vec<_>, _>>()?;

    let scan = async {
        let mut scanner = DeviceScanner::start(central, args.rssi, name_filter, args.device).await?;

        while let Some((peripheral, mut device)) = scanner.next_match().await? {
            let json = args.json;
            tokio::spawn(async move {
                match device.enumerate(&peripheral, &HashSet::new(), &HashSet::new()).await {
                    Ok(_) => {
                        device.read(&peripheral, &HashMap::new()).await?;
                        if json {
                            println!("{}", serde_json::to_string(&device)?)
                        } else {
                            print!("[+] Device: {}", device)
                        }
                        device.disconnect(&peripheral).await?;
                    }
                    Err(e) => eprintln!("Error: {e}"),
                }
                Ok::<(), anyhow::Error>(())
            });
        }
        Ok::<(), anyhow::Error>(())
    };

    run_with_timeout(args.timeout, args.json, scan).await
}
