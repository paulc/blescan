use anyhow::{Context, anyhow};
use btleplug::api::Manager as _;
use btleplug::platform::Manager;
use std::io::Read;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

mod characteristic_data;
mod commands;
mod dump;
mod enumerate;
mod event;
mod monitor;
mod notify;
mod poll;
mod scan;
mod scanner;
mod types;
mod util;
mod write;

use crate::commands::*;

// Include UUID maps
use crate::util::parse_uuid; // Needed for uuid_map include
include!(concat!(env!("OUT_DIR"), "/uuid_map.rs"));

// Default connection parameters
static CONNECT_TIMEOUT: AtomicU64 = AtomicU64::new(5);
static ENUMERATE_TIMEOUT: AtomicU64 = AtomicU64::new(5);
static WRITE_TIMEOUT: AtomicU64 = AtomicU64::new(5);
static DISCONNECT_TIMEOUT: AtomicU64 = AtomicU64::new(2);
static MAX_TASKS: AtomicUsize = AtomicUsize::new(10);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Get args
    let args: Args = argh::from_env();

    if args.dump_json {
        eprintln!("{}", serde_json::to_string_pretty(&args.command)?);
        return Ok(());
    }

    // Update connection params from args
    if let Some(t) = args.connect_timeout {
        CONNECT_TIMEOUT.store(t, Ordering::Relaxed)
    }
    if let Some(t) = args.enumerate_timeout {
        ENUMERATE_TIMEOUT.store(t, Ordering::Relaxed)
    }
    if let Some(t) = args.write_timeout {
        WRITE_TIMEOUT.store(t, Ordering::Relaxed)
    }
    if let Some(t) = args.disconnect_timeout {
        DISCONNECT_TIMEOUT.store(t, Ordering::Relaxed)
    }
    if let Some(t) = args.max_tasks {
        MAX_TASKS.store(t, Ordering::Relaxed)
    }

    // Initialise Bluetooth
    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;
    let central = adapters
        .into_iter()
        .next()
        .ok_or(anyhow!("No Bluetooth adapters found"))?;

    // Run command
    match args.command {
        Commands::Scan(args) => scan::run(central, args).await?,
        Commands::Enumerate(args) => enumerate::run(central, args).await?,
        Commands::Poll(args) => poll::run(central, args).await?,
        Commands::Notify(args) => notify::run(central, args).await?,
        Commands::Write(args) => write::run(central, args).await?,
        Commands::Dump(args) => dump::run(central, args).await?,
        Commands::Monitor(args) => monitor::run(central, args).await?,
        // Load command file and run
        Commands::Run(args) => match read_json_command(&args.path)? {
            Commands::Scan(args) => scan::run(central, args).await?,
            Commands::Enumerate(args) => enumerate::run(central, args).await?,
            Commands::Poll(args) => poll::run(central, args).await?,
            Commands::Notify(args) => notify::run(central, args).await?,
            Commands::Write(args) => write::run(central, args).await?,
            Commands::Dump(args) => dump::run(central, args).await?,
            Commands::Run(_) => anyhow::bail!("Invalid JSON command file: <run> not allowed"),
            Commands::Monitor(_) => anyhow::bail!("Invalid JSON command file: <monitorun> not allowed"),
        },
    }

    Ok(())
}

pub fn read_json_command(path: &str) -> anyhow::Result<Commands> {
    let json = if path == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        s
    } else {
        std::fs::read_to_string(path)?
    };
    let command: Commands = serde_json::from_str(&json).context("Invalid JSON command file")?;
    Ok(command)
}
