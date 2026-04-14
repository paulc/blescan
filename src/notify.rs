use anyhow::{anyhow, Context};
use btleplug::api::{
    bleuuid::BleUuid, Central, CentralEvent, CharPropFlags, Peripheral as _, ScanFilter,
};
use btleplug::platform::Adapter;
use futures::StreamExt;
use std::time::Duration;
use tokio::time::timeout;

use std::collections::HashSet;

use crate::types::{DeviceInfo, NotificationInfo};
use crate::util::parse_uuid;
use crate::NotifyArgs;
use crate::{CONNECT_TIMEOUT, DISCONNECT_TIMEOUT, ENUMERATE_TIMEOUT};

pub async fn run(central: Adapter, args: NotifyArgs) -> anyhow::Result<()> {
    // Convert args
    let service_match = parse_uuid(&args.service).context("Error Parsing Service UUID")?;
    let characteristic_match =
        parse_uuid(&args.characteristic).context("Error Parsing Characteristic UUID")?;
    if let Some(ref device) = args.device {
        parse_uuid(&device).context("Error Parsing Device UUID")?;
    }

    // ScanFilter only checks for services in the Advertisement
    // payload which tend to be very limited (31 bytes max) rather
    // then the full list of GATT services (which need connection)
    central.start_scan(ScanFilter::default()).await?;
    let mut seen = HashSet::new();

    let scan = async {
        let mut events = central.events().await?;
        while let Some(event) = events.next().await {
            // Check if the event is a device discovery
            match event {
                CentralEvent::DeviceDiscovered(id) => {
                    if seen.contains(&id) {
                        continue;
                    }
                    seen.insert(id.clone());
                    match central.peripheral(&id).await {
                        Ok(peripheral) => {
                            // Get basic info first (fast)
                            let device = DeviceInfo::new(&peripheral).await?;
                            // Filter by RSSI
                            if let Some(rssi) = args.rssi {
                                if device.rssi < rssi {
                                    continue;
                                }
                            }

                            // Filter by name
                            if args.name.as_ref().is_some_and(|name| *name != device.name) {
                                continue;
                            }

                            // Filter by device UUID
                            if args.device.as_ref().is_some_and(|id| *id != device.id) {
                                continue;
                            }

                            // Connect to device in background

                            tokio::spawn(async move {
                                match timeout(
                                    Duration::from_secs(CONNECT_TIMEOUT),
                                    peripheral.connect(),
                                )
                                .await
                                {
                                    Ok(Ok(_)) => {
                                        match timeout(
                                            Duration::from_secs(ENUMERATE_TIMEOUT),
                                            peripheral.discover_services(),
                                        )
                                        .await
                                        {
                                            Ok(Ok(_)) => {
                                                for service in peripheral.services() {
                                                    if service.uuid == service_match {
                                                        for characteristic in
                                                            &service.characteristics
                                                        {
                                                            if characteristic.uuid
                                                                == characteristic_match
                                                            {
                                                                if characteristic
                                                                    .properties
                                                                    .contains(CharPropFlags::NOTIFY)
                                                                {
                                                                    peripheral
                                                                        .subscribe(&characteristic)
                                                                        .await?;
                                                                    // Listen for notifications
                                                                    let mut notification_stream =
                                                                        peripheral
                                                                            .notifications()
                                                                            .await?;
                                                                    while let Some(notification) =
                                                                        notification_stream
                                                                            .next()
                                                                            .await
                                                                    {
                                                                        let n = NotificationInfo {
                                                                            service: notification.service_uuid.to_short_string(),
                                                                            characteristic: notification.uuid.to_short_string(),
                                                                            value: notification.value
                                                                        };
                                                                        if args.json {
                                                                            println!("{}", serde_json::to_string(&n)?);
                                                                        } else {
                                                                            println!("{}", n);
                                                                        }
                                                                    }
                                                                } else {
                                                                    eprintln!("ERROR: Notify not supported [{}]", service.uuid);
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            _ => {
                                                eprintln!(
                                                    "Service discovery failed/timeout for {}",
                                                    peripheral.id()
                                                )
                                            }
                                        }
                                        if let Err(e) = timeout(
                                            Duration::from_secs(DISCONNECT_TIMEOUT),
                                            peripheral.disconnect(),
                                        )
                                        .await
                                        {
                                            eprintln!(
                                                "Disconnect timeout/error for {}: {:?}",
                                                peripheral.id(),
                                                e
                                            );
                                        }
                                    }
                                    _ => {
                                        eprintln!("Connect timeout/error for {}", peripheral.id())
                                    }
                                }
                                Ok::<(), anyhow::Error>(())
                            });
                        }
                        Err(e) => {
                            eprintln!("Error retrieving peripheral: {:?}", e);
                        }
                    }
                }
                _ => {
                    // eprintln!(">> EVENT: {:?}", event);
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    };

    if let Some(t) = args.timeout {
        if !args.json {
            println!("Listening for BLE advertisements: Timeout {t} secs");
        }
        match timeout(Duration::from_secs(t), scan).await {
            Ok(result) => result.map_err(|e| anyhow!("Scan Error: {e}"))?,
            Err(_) => println!("\n[!] Timeout reached. Stopping scan."),
        }
    } else {
        if !args.json {
            println!("Listening for BLE advertisements: Ctrl+C to stop");
        }
        scan.await.map_err(|e| anyhow!("Scan Error: {e}"))?
    }

    Ok(())
}
