use anyhow::anyhow;
use btleplug::api::Manager as _;
use btleplug::platform::Manager;

mod characteristic_data;
mod commands;
mod dump;
mod enumerate;
mod filter;
mod notify;
mod poll;
mod scan;
mod types;
mod util;
mod write;

use crate::commands::*;

// Include UUID maps
use crate::util::parse_uuid; // Needed for uuid_map include
include!(concat!(env!("OUT_DIR"), "/uuid_map.rs"));

pub const ENUMERATE_TIMEOUT: u64 = 5;
pub const CONNECT_TIMEOUT: u64 = 5;
pub const WRITE_TIMEOUT: u64 = 5;
pub const DISCONNECT_TIMEOUT: u64 = 1;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Get args
    let args: Args = argh::from_env();

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
    }

    Ok(())
}
