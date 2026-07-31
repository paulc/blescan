use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};

use btleplug::platform::Adapter;
use tokio::signal::ctrl_c;

use rquickjs::{AsyncContext, AsyncRuntime};
use rquickjs_utils::repl::repl_rl;
use rquickjs_utils::run::{call_fn, run_script, set_resolve_promise};
use rquickjs_utils::{
    date, fetch, utils,
    utils::{json_to_value, log_v},
};

#[cfg(feature = "mqtt")]
use rquickjs_utils::channel::{register_mpsc_rx, register_mpsc_rx_cb, register_mpsc_tx};
#[cfg(feature = "mqtt")]
use rquickjs_utils::mqtt_task::{MqttConfig, MqttTask};
#[cfg(feature = "mqtt")]
use rquickjs_utils::mqtt_types::{register_mqtt, MqttCommand, MqttEvent};
#[cfg(feature = "mqtt")]
use tokio::sync::mpsc;

use crate::bridge;
use crate::commands::JsArgs;

static USER_EXIT: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "mqtt")]
async fn start_mqtt(
    args: JsArgs,
) -> anyhow::Result<(mpsc::UnboundedSender<MqttCommand>, mpsc::UnboundedReceiver<MqttEvent>)> {
    // Create MQTT configuration
    let config = MqttConfig {
        broker_addr: args.address,
        broker_port: args.port,
        client_id: args
            .client_id
            .unwrap_or(format!("mqtt_client_{}", uuid::Uuid::new_v4())),
        username: args.username,
        password: args.password,
        ..Default::default()
    };
    // Start the MQTT task and return Sender/Receiver
    MqttTask::new(config)
        .start()
        .await
        .map_err(|e| anyhow::anyhow!("MqttTask: {e}"))
}

pub async fn run(central: Adapter, args: JsArgs) -> anyhow::Result<()> {
    tokio::spawn(async move {
        ctrl_c().await.expect("Error listening for Ctrl-C");
        eprintln!("[+] User Exit",);
        USER_EXIT.store(true, Ordering::Relaxed);
    });

    // Start MQTT connection if mqtt feature & cli option enabled
    #[cfg(feature = "mqtt")]
    let mqtt_chans = if args.mqtt {
        Some(start_mqtt(args.clone()).await?)
    } else {
        None
    };

    let rt = AsyncRuntime::new()?;
    let ctx = AsyncContext::full(&rt).await?;

    // Set interrupt handler - this only seems to be called on ctx.eval() so not actually useful
    rt.set_interrupt_handler(Some(Box::new(|| USER_EXIT.load(Ordering::Relaxed))))
        .await;

    ctx.async_with(async |ctx| {
        // Install default fns in ctx
        utils::register_fns(&ctx)?;
        date::register_date(&ctx)?;
        fetch::register_fetch(&ctx)?;

        // Install BLE scan
        bridge::install_scan(&ctx, central)?;

        #[cfg(feature = "mqtt")]
        {
            // Register MQTT objects if cli option enabled
            if let Some((command_tx, event_rx)) = mqtt_chans {
                register_mqtt(&ctx)?;
                register_mpsc_tx(ctx.clone(), command_tx, "mqtt_tx")?;
                if args.cb {
                    register_mpsc_rx_cb(ctx.clone(), event_rx, "mqtt_rx_cb")?;
                } else {
                    register_mpsc_rx(ctx.clone(), event_rx, "mqtt_rx")?;
                }
            }
        }

        set_resolve_promise(&ctx, args.resolve_promise)?;

        // Run JS files
        for file in args.file {
            run_script(ctx.clone(), get_file(&file)?).await?;
        }

        // Run JS script literals
        for script in args.script {
            run_script(ctx.clone(), script).await?;
        }

        // Call JS fn
        for (f, a) in args
            .call
            .iter()
            .zip(args.arg.iter().chain(std::iter::repeat(&("".to_string()))))
        {
            // Resolves future if __resolve_promise set
            let r = if a.is_empty() {
                call_fn(ctx.clone(), &f, ((),)).await?
            } else {
                call_fn(ctx.clone(), &f, (json_to_value(ctx.clone(), a)?,)).await?
            };
            eprintln!("[+] Call: {f}({a}) => {}", log_v(&r, false, 0));
        }

        if args.repl {
            repl_rl(ctx.clone()).await?;
        }

        Ok::<(), anyhow::Error>(())
    })
    .await?;

    eprintln!("[+] Tasks Pending: {:?}", rt.is_job_pending().await);

    while rt.is_job_pending().await && !USER_EXIT.load(Ordering::Relaxed) {
        rt.execute_pending_job()
            .await
            .map_err(|_| anyhow::anyhow!("JS Runtime Error"))?;
        tokio::task::yield_now().await;
    }

    Ok(())
}

fn get_file(file: &str) -> anyhow::Result<String> {
    Ok(if file == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        s
    } else {
        std::fs::read_to_string(file)?
    })
}
