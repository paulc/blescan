use btleplug::api::{Central, CentralEvent, ScanFilter};
use btleplug::platform::{Adapter, Peripheral, PeripheralId};
use futures::Stream;
use futures::stream::StreamExt;
use regex::Regex;
use uuid::Uuid;

use std::collections::HashSet;
use std::pin::Pin;

use crate::types::DeviceInfo;

/// Wraps BLE event stream and filtering logic
pub struct DeviceScanner {
    central: Adapter,
    events: Pin<Box<dyn Stream<Item = CentralEvent> + Send>>,
    seen: HashSet<PeripheralId>,
    rssi_filter: Option<i16>,
    name_filter: Vec<Regex>,
    device_filter: Vec<Uuid>,
}

impl DeviceScanner {
    /// Create scanner
    pub async fn start(
        central: Adapter,
        rssi_filter: Option<i16>,
        name_filter: Vec<Regex>,
        device_filter: Vec<Uuid>,
    ) -> anyhow::Result<Self> {
        // ScanFilter only checks for services in the Advertisement payload
        // rather then the full list of GATT services (which need connection)
        central.start_scan(ScanFilter::default()).await?;
        let events = central.events().await?;

        Ok(Self {
            central,
            events: Box::pin(events),
            seen: HashSet::new(),
            rssi_filter,
            name_filter,
            device_filter,
        })
    }

    /// Pull the next event and returns it if it passes all device filters
    pub async fn next_match(&mut self) -> anyhow::Result<Option<(Peripheral, DeviceInfo)>> {
        while let Some(event) = self.events.next().await {
            if let CentralEvent::DeviceDiscovered(id) = event {
                if self.seen.contains(&id) {
                    continue;
                }
                self.seen.insert(id.clone());
                if let Ok(peripheral) = self.central.peripheral(&id).await
                    && let Ok(device) = DeviceInfo::new(&peripheral).await
                    && self.rssi_filter.is_none_or(|rssi| device.rssi >= rssi)
                    && (self.name_filter.is_empty() || self.name_filter.iter().any(|r| r.is_match(&device.name)))
                    && (self.device_filter.is_empty() || self.device_filter.contains(&device.id))
                {
                    return Ok(Some((peripheral, device)));
                }
            }
        }
        Ok(None) // Stream ended
    }
}
