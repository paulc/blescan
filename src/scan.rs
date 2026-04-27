use btleplug::platform::Adapter;

use crate::commands::ScanArgs;
use crate::scanner::DeviceScanner;
use crate::util::{make_regex_filter, run_with_timeout};

pub async fn run(central: Adapter, args: ScanArgs) -> anyhow::Result<()> {
    let device_filter = args.device;
    let name_filter = make_regex_filter(&args.name)?;
    let scan = async {
        let mut scanner = DeviceScanner::start(central, args.rssi, name_filter, device_filter, !args.show_seen).await?;
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
