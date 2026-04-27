use btleplug::platform::Adapter;
use crossterm::{
    cursor::MoveTo,
    event::EventStream,
    execute,
    terminal::{Clear, ClearType, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;

use std::collections::HashMap;
use std::io::stdout;

use crate::commands::MonitorArgs;
use crate::scanner::DeviceScanner;
use crate::types::DeviceInfo;
use crate::util::run_with_timeout;

pub async fn run(central: Adapter, args: MonitorArgs) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = stdout();
    let scan = async {
        let mut seen: HashMap<String, DeviceInfo> = HashMap::new();
        let mut scanner = DeviceScanner::start(central, None, Vec::new(), Vec::new(), false).await?;
        let mut reader = EventStream::new();
        loop {
            tokio::select! {
                event = reader.next() => {
                    println!("{:?}",event);
                    break
                },
                Ok(Some((_peripheral, device))) = scanner.next_match() => {
                    seen.insert(device.id.clone(), device.clone());
                    let mut devices = seen.values().collect::<Vec<_>>();
                    devices.sort_by_key(|d| -d.rssi.unwrap_or(i16::MIN));
                    execute!(stdout, Clear(ClearType::All), MoveTo(0, 0))?;
                    for d in &devices {
                        print!("{}\r", d);
                    }
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    };
    match run_with_timeout(args.timeout, false, scan).await {
        _ => {}
    };
    disable_raw_mode()?;
    Ok(())
}
