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

use crate::bridge;
use crate::commands::JsArgs;

static USER_EXIT: AtomicBool = AtomicBool::new(false);

pub async fn run(central: Adapter, args: JsArgs) -> anyhow::Result<()> {
    tokio::spawn(async move {
        ctrl_c().await.expect("Error listening for Ctrl-C");
        eprintln!("[+] User Exit",);
        USER_EXIT.store(true, Ordering::Relaxed);
    });

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
