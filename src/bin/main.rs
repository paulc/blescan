use anyhow::{Context, anyhow};
use btleplug::api::Manager as _;
use btleplug::platform::Manager;
use std::io::Read;
use std::sync::atomic::Ordering;

use blescan::commands::*;
#[cfg(feature = "quickjs")]
use blescan::js;
use blescan::{CONNECT_TIMEOUT, DISCONNECT_TIMEOUT, ENUMERATE_TIMEOUT, MAX_TASKS, WRITE_TIMEOUT};
use blescan::{dump, enumerate, monitor, notify, poll, scan, write, write_read};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Check for --version before parsing args to avoid argh complaining
    if std::env::args().skip(1).any(|arg| arg == "--version") {
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

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

    // SSE server forks a child process per request so doesnt need Bluetooth
    #[cfg(feature = "sse")]
    if let Commands::Sse(sse_args) = args.command {
        let mut global_args = vec![];
        for (k, v) in [
            ("--connect-timeout", args.connect_timeout),
            ("--enumerate-timeout", args.enumerate_timeout),
            ("--write-timeout", args.write_timeout),
            ("--disconnect-timeout", args.disconnect_timeout),
            ("--max-tasks", args.max_tasks.map(|v| v as u64)),
        ] {
            if let Some(v) = v {
                global_args.extend([k.to_string(), v.to_string()]);
            }
        }
        return blescan::sse::run(sse_args, global_args).await;
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
        Commands::WriteRead(args) => write_read::run(central, args).await?,
        Commands::Dump(args) => dump::run(central, args).await?,
        Commands::Monitor(args) => monitor::run(central, args).await?,
        // Load command file and run
        Commands::Run(args) => match read_json_command(&args.path)? {
            Commands::Scan(args) => scan::run(central, args).await?,
            Commands::Enumerate(args) => enumerate::run(central, args).await?,
            Commands::Poll(args) => poll::run(central, args).await?,
            Commands::Notify(args) => notify::run(central, args).await?,
            Commands::Write(args) => write::run(central, args).await?,
            Commands::WriteRead(args) => write_read::run(central, args).await?,
            Commands::Dump(args) => dump::run(central, args).await?,
            Commands::Monitor(args) => monitor::run(central, args).await?,
            #[cfg(feature = "quickjs")]
            Commands::Js(args) => js::run(central, args).await?,
            Commands::Run(_) => anyhow::bail!("Invalid JSON command file: <run> not allowed"),
            #[cfg(feature = "sse")]
            Commands::Sse(_) => anyhow::bail!("Invalid JSON command file: <sse> not allowed"),
        },
        #[cfg(feature = "quickjs")]
        Commands::Js(args) => {
            js::run(central, args).await?;
        }
        #[cfg(feature = "sse")]
        Commands::Sse(_) => unreachable!(),
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
