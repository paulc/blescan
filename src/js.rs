use std::sync::atomic::{AtomicBool, Ordering};

use btleplug::platform::Adapter;
use tokio::signal::ctrl_c;

use rquickjs::{AsyncContext, AsyncRuntime};
use rquickjs_utils::repl::repl_rl;
use rquickjs_utils::utils;

use crate::bridge;
use crate::commands::JsArgs;

static USER_EXIT: AtomicBool = AtomicBool::new(false);

pub async fn run(central: Adapter, _args: JsArgs) -> anyhow::Result<()> {
    tokio::spawn(async move {
        ctrl_c().await.expect("Error listening for Ctrl-C");
        println!("[+] User Exit",);
        USER_EXIT.store(true, Ordering::Relaxed);
    });

    let rt = AsyncRuntime::new()?;
    let ctx = AsyncContext::full(&rt).await?;

    // Set interrupt handler - this only seems to be called on ctx.eval() so not actually useful
    rt.set_interrupt_handler(Some(Box::new(|| USER_EXIT.load(Ordering::Relaxed))))
        .await;

    ctx.async_with(async |ctx| {
        utils::register_fns(&ctx)?;
        bridge::install_scan(&ctx, central)?;
        repl_rl(ctx.clone()).await?;
        Ok::<(), anyhow::Error>(())
    })
    .await?;

    println!("[+] Tasks Pending: {:?}", rt.is_job_pending().await);

    while rt.is_job_pending().await && !USER_EXIT.load(Ordering::Relaxed) {
        rt.execute_pending_job()
            .await
            .map_err(|_| anyhow::anyhow!("JS Runtime Error"))?;
        tokio::task::yield_now().await;
    }

    Ok(())
}
