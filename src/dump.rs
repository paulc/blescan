use crate::commands::DumpArgs;
use btleplug::api::{Central, ScanFilter};
use btleplug::platform::Adapter;
use futures::StreamExt;
use tokio::time::timeout;

use std::collections::HashSet;
use std::time::Duration;

use crate::event::{EventFilter, EventWrapper};
use crate::util::make_regex_filter;

pub async fn run(central: Adapter, args: DumpArgs) -> anyhow::Result<()> {
    let event_filter = args
        .event
        .iter()
        .map(|s| EventFilter::try_from(s.as_str()))
        .collect::<Result<HashSet<_>, _>>()?;
    let name_filter = make_regex_filter(&args.name)?;

    central.start_scan(ScanFilter::default()).await?;

    let scan = async {
        let mut events = central.events().await?;
        while let Some(event) = events.next().await {
            let event = EventWrapper(event);
            // Event filter
            if !event_filter.is_empty() && !event_filter.iter().any(|f| event.filter(f)) {
                continue;
            }
            // Device filter
            if !args.device.is_empty() && event.id().is_some_and(|e| !args.device.contains(&e.to_string())) {
                continue;
            }
            let event_info = event.get_event_info(&central).await?;
            // Name filter
            if !name_filter.is_empty()
                && !event_info
                    .get_name()
                    .is_some_and(|name| name_filter.iter().any(|r| r.is_match(name)))
            {
                continue;
            }
            if args.json {
                println!("{}", serde_json::to_string(&event_info)?);
            } else {
                print!("[+] {}", event_info);
            }
        }
        Ok::<(), anyhow::Error>(())
    };

    if let Some(t) = args.timeout {
        if !args.json {
            eprintln!("Dumping BLE advertisements: Timeout {t} secs");
        }
        if timeout(Duration::from_secs(t), scan).await.is_err() && !args.json {
            eprintln!("[-] Timeout");
        }
    } else {
        if !args.json {
            eprintln!("Dumping BLE advertisements: Ctrl+C to stop");
        }
        scan.await?;
    }
    Ok(())
}
