use anyhow::Context;
use btleplug::api::CharPropFlags;
use uuid::Uuid;

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::sync::Arc;

use crate::characteristic_data::{CharData, CharFormat};

pub fn format_properties(props: &CharPropFlags) -> String {
    let mut p = Vec::new();
    if props.contains(CharPropFlags::BROADCAST) {
        p.push("Broadcast");
    }
    if props.contains(CharPropFlags::READ) {
        p.push("Read");
    }
    if props.contains(CharPropFlags::WRITE) {
        p.push("Write");
    }
    if props.contains(CharPropFlags::WRITE_WITHOUT_RESPONSE) {
        p.push("WriteNoResp");
    }
    if props.contains(CharPropFlags::NOTIFY) {
        p.push("Notify");
    }
    if props.contains(CharPropFlags::INDICATE) {
        p.push("Indicate");
    }
    if props.contains(CharPropFlags::AUTHENTICATED_SIGNED_WRITES) {
        p.push("AuthSignedWrite");
    }
    format!("[{}]", p.join(","))
}

pub fn parse_uuid(s: &str) -> Result<uuid::Uuid, uuid::Error> {
    if s.len() == 4 {
        // 16-bit UUID
        let full = format!("0000{}-0000-1000-8000-00805f9b34fb", s.to_lowercase());
        uuid::Uuid::parse_str(&full)
    } else if s.len() == 6 && s.starts_with("0x") {
        // 16-bit UUID (0x prefix)
        let s = &s[2..];
        let full = format!("0000{}-0000-1000-8000-00805f9b34fb", s.to_lowercase());
        uuid::Uuid::parse_str(&full)
    } else if s.len() == 8 {
        // 32-bit UUID
        let full = format!("{}-0000-1000-8000-00805f9b34fb", s.to_lowercase());
        uuid::Uuid::parse_str(&full)
    } else if s.len() == 10 && s.starts_with("0x") {
        // 32-bit UUID (0x prefix)
        let s = &s[2..];
        let full = format!("{}-0000-1000-8000-00805f9b34fb", s.to_lowercase());
        uuid::Uuid::parse_str(&full)
    } else {
        uuid::Uuid::parse_str(s)
    }
}

pub fn uuid_filter(filters: &[String]) -> anyhow::Result<Arc<HashSet<Uuid>>> {
    Ok(Arc::new(
        filters
            .iter()
            .map(|s| parse_uuid(s))
            .collect::<Result<HashSet<Uuid>, _>>()
            .context("Error Parsing UUID")?,
    ))
}

pub fn read_all_lines(files: &[String]) -> anyhow::Result<Vec<String>> {
    let mut result = Vec::new();
    for f in files {
        let file = File::open(f)?;
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            // Skip commands & blank lines
            if !(line.trim().is_empty() || line.starts_with("#")) {
                result.push(line.trim().to_string());
            }
        }
    }
    Ok(result)
}

pub fn parse_decoder(decoders: &[String]) -> anyhow::Result<Arc<HashMap<Uuid, CharFormat>>> {
    Ok(Arc::new(
        decoders
            .iter()
            .map(|s| {
                s.split_once("::").context("Invalid Format").and_then(|(uuid, fmt)| {
                    let uuid = parse_uuid(uuid)?;
                    let fmt = CharFormat::try_from(fmt)?;
                    Ok((uuid, fmt))
                })
            })
            .collect::<Result<HashMap<_, _>, _>>()
            .context("Error Parsing Decode Mapping")?,
    ))
}

pub fn parse_write(characteristics: &[String]) -> anyhow::Result<Arc<HashMap<Uuid, Vec<u8>>>> {
    Ok(Arc::new(
        characteristics
            .iter()
            .map(|s| {
                s.split_once("::").context("Invalid Format").and_then(|(uuid, data)| {
                    let uuid = parse_uuid(uuid)?;
                    let data = CharData::try_from(data)?.to_vec().clone();
                    Ok((uuid, data))
                })
            })
            .collect::<Result<HashMap<_, _>, _>>()
            .context("Error Parsing Write Data")?,
    ))
}

pub async fn run_with_timeout<F>(timeout_secs: Option<u64>, json: bool, task: F) -> anyhow::Result<()>
where
    F: std::future::Future<Output = anyhow::Result<()>>,
{
    if let Some(t) = timeout_secs {
        if !json {
            println!("Listening for BLE advertisements: Timeout {t} secs");
        }
        match tokio::time::timeout(std::time::Duration::from_secs(t), task).await {
            Ok(result) => result.map_err(|e| anyhow::anyhow!("Scan Error: {e}")),
            Err(_) => {
                if !json {
                    println!("\n[!] Timeout reached. Stopping scan.");
                }
                Ok(())
            }
        }
    } else {
        if !json {
            println!("Listening for BLE advertisements: Ctrl+C to stop");
        }
        task.await.map_err(|e| anyhow::anyhow!("Scan Error: {e}"))
    }
}
