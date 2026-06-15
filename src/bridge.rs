//! JavaScript bridge for the BLE scanner.
//!
//! Installs a global `scan(opts)` into a QuickJS (`rquickjs`) async context.
//! `scan` returns a *breakable async-iterable* of device objects; each device
//! object exposes the `DeviceInfo` operations as async methods and carries its
//! own `Peripheral` handle.
//!
//! ```js
//! const decode = ["2a37::u8,u8"];   // set from CLI args if needed
//! for await (const dev of scan({ rssi: -70, names: ["^Polar"], filter_seen: true })) {
//!     await dev.connect();
//!     await dev.enumerate();             // optional: ([serviceUuid...], [charUuid...])
//!     await dev.read(decode);            // decode arg optional; omit for raw reads
//!     console.log(await dev.snapshot()); // full DeviceInfo with values/decoded
//!     await dev.write(svcUuid, charUuid, "01ff");
//!     await dev.disconnect();
//!     break;                             // stops the radio scan
//! }
//! ```
//!
//! Design notes:
//! * `scan()` returns the iterable synchronously; `DeviceScanner::start` is
//!   deferred to the first `next()`, so start failures flow through the same
//!   error path as iteration (and a bad regex throws at the `scan()` call site).
//! * `break` works because the iterator implements `return()`, which stops the
//!   scan via a cloned `Adapter` (never contending for the scanner lock).
//! * Each device wraps `Arc<futures::lock::Mutex<DeviceInfo>>` + a cloned
//!   `Peripheral`, so async methods never hold a QuickJS class borrow across an
//!   `.await`. Per-device operations are serialized; distinct devices are
//!   independent.
//! * Async methods resolve to a `JsOutcome` -> `[value, error]` JS array
//!   (rquickjs has no `IntoJs` for tuples, hence the explicit carrier). The JS
//!   shims throw on the error slot and `JSON.parse` the value slot.
//!
//! Adjust the `use crate::...` paths below to match your module layout.

use std::sync::Arc;

use btleplug::api::Central;
use btleplug::platform::{Adapter, Peripheral};
use futures::lock::Mutex;
use regex::Regex;
use rquickjs::{
    function::{Async, Opt},
    Array, Ctx, FromJs, Function, IntoJs, Object, Result as JsResult, Value,
};

use crate::scanner::DeviceScanner;
use crate::types::DeviceInfo;
use crate::util::{make_regex_filter, make_uuid_filter, parse_decoder, parse_uuid};

// ===========================================================================
// Result carrier: [value, error] (tuples don't implement IntoJs)
// ===========================================================================

struct JsOutcome<T> {
    value: Option<T>,
    error: Option<String>,
}

impl<T> JsOutcome<T> {
    /// Success with a value.
    fn ok(value: T) -> Self {
        Self {
            value: Some(value),
            error: None,
        }
    }
    /// Success with no value (void methods).
    fn empty() -> Self {
        Self {
            value: None,
            error: None,
        }
    }
    /// Failure carrying a message that the JS shim rethrows.
    fn err(msg: String) -> Self {
        Self {
            value: None,
            error: Some(msg),
        }
    }
}

impl<'js, T: IntoJs<'js>> IntoJs<'js> for JsOutcome<T> {
    fn into_js(self, ctx: &Ctx<'js>) -> JsResult<Value<'js>> {
        let arr = Array::new(ctx.clone())?;
        arr.set(0, self.value)?; // Option<T> -> value | null
        arr.set(1, self.error)?; // Option<String> -> string | null
        Ok(arr.into_value())
    }
}

// ===========================================================================
// Public entry point
// ===========================================================================

/// Install a global `scan(opts)` function into `ctx`.
///
/// `opts` (all fields optional):
/// `{ rssi?: number, names?: string[], devices?: string[], filter_seen?: bool }`.
/// `names` are regular expressions; an invalid pattern throws synchronously.
pub fn install_scan<'js>(ctx: &Ctx<'js>, central: Adapter) -> JsResult<()> {
    let scan = Function::new(
        ctx.clone(),
        // NOTE: tie the ctx param and the returned Object to the SAME `'js`,
        // otherwise the closure infers two independent lifetimes.
        move |ctx: Ctx<'js>, opts: ScanOpts| -> JsResult<Object<'js>> {
            // Compile name regexes eagerly so bad patterns fail at the call site.
            let names = match make_regex_filter(&opts.names) {
                Ok(n) => n,
                Err(e) => return Err(ctx.throw(e.to_string().into_js(&ctx)?)),
            };
            build_scan_iterable(&ctx, central.clone(), opts.rssi, names, opts.devices, opts.filter_seen)
        },
    )?;
    ctx.globals().set("scan", scan)?;
    Ok(())
}

// ===========================================================================
// Scan options
// ===========================================================================

struct ScanOpts {
    rssi: Option<i16>,
    names: Vec<String>,
    devices: Vec<String>,
    filter_seen: bool,
}

impl<'js> FromJs<'js> for ScanOpts {
    fn from_js(_ctx: &Ctx<'js>, value: Value<'js>) -> JsResult<Self> {
        // A missing / non-object argument => all defaults.
        let Some(o) = value.into_object() else {
            return Ok(ScanOpts {
                rssi: None,
                names: Vec::new(),
                devices: Vec::new(),
                filter_seen: false,
            });
        };
        // Accept name/names & device/devices
        let names = match o.get::<_, Option<Vec<String>>>("names")? {
            Some(v) => v,
            None => o.get::<_, Option<String>>("name")?.map(|s| vec![s]).unwrap_or_default(),
        };
        let devices = match o.get::<_, Option<Vec<String>>>("devices")? {
            Some(v) => v,
            None => o
                .get::<_, Option<String>>("device")?
                .map(|s| vec![s])
                .unwrap_or_default(),
        };
        Ok(ScanOpts {
            rssi: o.get::<_, Option<i16>>("rssi")?,
            names,
            devices,
            filter_seen: o.get::<_, Option<bool>>("filter_seen")?.unwrap_or(false),
        })
    }
}

// ===========================================================================
// Breakable async-iterable over scanned devices
// ===========================================================================

enum ScanState {
    Idle {
        rssi: Option<i16>,
        names: Vec<Regex>,
        devices: Vec<String>,
        filter_seen: bool,
    },
    Running(DeviceScanner),
    Closed,
}

fn build_scan_iterable<'js>(
    ctx: &Ctx<'js>,
    central: Adapter,
    rssi: Option<i16>,
    names: Vec<Regex>,
    devices: Vec<String>,
    filter_seen: bool,
) -> JsResult<Object<'js>> {
    let state = Arc::new(Mutex::new(ScanState::Idle {
        rssi,
        names,
        devices,
        filter_seen,
    }));

    // next(): lazily start on first pull, then yield one match.
    // Resolves to [device|null, errMsg|null].
    let n_central = central.clone();
    let n_state = state.clone();
    let rust_next = Function::new(
        ctx.clone(),
        Async(move || {
            let central = n_central.clone();
            let state = n_state.clone();
            async move {
                let mut guard = state.lock().await;

                // First pull: consume the Idle params and start the scan.
                if matches!(&*guard, ScanState::Idle { .. }) {
                    let ScanState::Idle {
                        rssi,
                        names,
                        devices,
                        filter_seen,
                    } = std::mem::replace(&mut *guard, ScanState::Closed)
                    else {
                        unreachable!()
                    };
                    match DeviceScanner::start(central, rssi, names, devices, filter_seen).await {
                        Ok(s) => *guard = ScanState::Running(s),
                        Err(e) => return JsOutcome::err(e.to_string()), // stays Closed
                    }
                }

                match &mut *guard {
                    ScanState::Running(s) => match s.next_match().await {
                        Ok(Some((peripheral, info))) => JsOutcome::ok(ScannedDevice { peripheral, info }),
                        Ok(None) => JsOutcome::empty(),
                        Err(e) => JsOutcome::err(e.to_string()),
                    },
                    _ => JsOutcome::empty(), // Closed
                }
            }
        }),
    )?;

    // return(): stop the radio scan (if running) and close. Idempotent.
    let r_central = central.clone();
    let r_state = state.clone();
    let rust_return = Function::new(
        ctx.clone(),
        Async(move || {
            let central = r_central.clone();
            let state = r_state.clone();
            async move {
                let prev = std::mem::replace(&mut *state.lock().await, ScanState::Closed);
                if let ScanState::Running(_) = prev {
                    if let Err(e) = central.stop_scan().await {
                        return Some(e.to_string());
                    }
                }
                None::<String> // dropping `prev` drops the event stream
            }
        }),
    )?;

    let factory: Function = ctx.eval(
        r#"(rustNext, rustReturn) => ({
            async next() {
                const [item, err] = await rustNext();
                if (err) throw new Error(err);
                return item == null
                    ? { value: undefined, done: true }
                    : { value: item, done: false };
            },
            async return(value) {
                const err = await rustReturn();
                if (err) throw new Error(err);
                return { value, done: true };
            },
            // Explicit stop iterator
            async close() {
                const err = await rustReturn();
                if (err) throw new Error(err);
            },
            [Symbol.asyncIterator]() { return this; },
        })"#,
    )?;

    factory.call((rust_next, rust_return))
}

// ===========================================================================
// Device object
// ===========================================================================

/// Yielded by the iterator; `IntoJs` builds the device object at resolve time
/// (when a `Ctx` is available).
struct ScannedDevice {
    peripheral: Peripheral,
    info: DeviceInfo,
}

impl<'js> IntoJs<'js> for ScannedDevice {
    fn into_js(self, ctx: &Ctx<'js>) -> JsResult<Value<'js>> {
        Ok(device_into_js(ctx, self.peripheral, self.info)?.into_value())
    }
}

/// Build a JS object wrapping a `DeviceInfo` + its `Peripheral`.
///
/// Every method returns a Promise; rejection carries the Rust error message.
/// Mutating methods (`read`, `enumerate`, `write`, `updateRssi`) change internal
/// state that becomes visible through a subsequent `snapshot()`.
fn device_into_js<'js>(ctx: &Ctx<'js>, peripheral: Peripheral, info: DeviceInfo) -> JsResult<Object<'js>> {
    let id = info.id.clone();
    let inner = Arc::new(Mutex::new(info));

    // snapshot() -> current DeviceInfo as JSON
    let snapshot = {
        let inner = inner.clone();
        Function::new(
            ctx.clone(),
            Async(move || {
                let inner = inner.clone();
                async move {
                    match serde_json::to_string(&*inner.lock().await) {
                        Ok(j) => JsOutcome::ok(j),
                        Err(e) => JsOutcome::err(e.to_string()),
                    }
                }
            }),
        )?
    };

    // connect()
    let connect = {
        let (p, inner) = (peripheral.clone(), inner.clone());
        Function::new(
            ctx.clone(),
            Async(move || {
                let (p, inner) = (p.clone(), inner.clone());
                async move {
                    match inner.lock().await.connect(&p).await {
                        Ok(()) => JsOutcome::<String>::empty(),
                        Err(e) => JsOutcome::err(e.to_string()),
                    }
                }
            }),
        )?
    };

    // disconnect()
    let disconnect = {
        let (p, inner) = (peripheral.clone(), inner.clone());
        Function::new(
            ctx.clone(),
            Async(move || {
                let (p, inner) = (p.clone(), inner.clone());
                async move {
                    match inner.lock().await.disconnect(&p).await {
                        Ok(()) => JsOutcome::<String>::empty(),
                        Err(e) => JsOutcome::err(e.to_string()),
                    }
                }
            }),
        )?
    };

    // enumerate(services?, characteristics?) -- arrays of UUID strings (16/32-bit shorthand ok)
    let enumerate = {
        let (p, inner) = (peripheral.clone(), inner.clone());
        Function::new(
            ctx.clone(),
            Async(move |services: Opt<Vec<String>>, chars: Opt<Vec<String>>| {
                let (p, inner) = (p.clone(), inner.clone());
                async move {
                    let svc_list = services.0.unwrap_or_default();
                    let chr_list = chars.0.unwrap_or_default();
                    let sf = match make_uuid_filter(&svc_list) {
                        Ok(s) => s,
                        Err(e) => return JsOutcome::<String>::err(e.to_string()),
                    };
                    let cf = match make_uuid_filter(&chr_list) {
                        Ok(s) => s,
                        Err(e) => return JsOutcome::err(e.to_string()),
                    };
                    match inner.lock().await.enumerate(&p, &sf, &cf).await {
                        Ok(()) => JsOutcome::empty(),
                        Err(e) => JsOutcome::err(e.to_string()),
                    }
                }
            }),
        )?
    };

    // read(decode?) -- optional ["uuid::fmt"] decode rules, parsed per call
    let read = {
        let (p, inner) = (peripheral.clone(), inner.clone());
        Function::new(
            ctx.clone(),
            Async(move |decode: Opt<Vec<String>>| {
                let (p, inner) = (p.clone(), inner.clone());
                async move {
                    let rules = decode.0.unwrap_or_default();
                    let map = match parse_decoder(&rules) {
                        Ok(m) => m,
                        Err(e) => return JsOutcome::<String>::err(e.to_string()),
                    };
                    match inner.lock().await.read(&p, &map).await {
                        Ok(()) => JsOutcome::empty(),
                        Err(e) => JsOutcome::err(e.to_string()),
                    }
                }
            }),
        )?
    };

    // subscribe() -> Vec<SubscriptionInfo> as JSON
    let subscribe = {
        let (p, inner) = (peripheral.clone(), inner.clone());
        Function::new(
            ctx.clone(),
            Async(move || {
                let (p, inner) = (p.clone(), inner.clone());
                async move {
                    match inner.lock().await.subscribe(&p).await {
                        Ok(subs) => match serde_json::to_string(&subs) {
                            Ok(j) => JsOutcome::ok(j),
                            Err(e) => JsOutcome::err(e.to_string()),
                        },
                        Err(e) => JsOutcome::err(e.to_string()),
                    }
                }
            }),
        )?
    };

    // write(service, characteristic, hexValue, withoutResponse?)
    let write = {
        let (p, inner) = (peripheral.clone(), inner.clone());
        Function::new(
            ctx.clone(),
            Async(
                move |service: String, characteristic: String, value: String, wor: Opt<bool>| {
                    let (p, inner) = (p.clone(), inner.clone());
                    async move {
                        let svc = match parse_uuid(&service) {
                            Ok(u) => u,
                            Err(e) => return JsOutcome::<String>::err(format!("bad service uuid: {e}")),
                        };
                        let chr = match parse_uuid(&characteristic) {
                            Ok(u) => u,
                            Err(e) => return JsOutcome::err(format!("bad characteristic uuid: {e}")),
                        };
                        let bytes = match hex::decode(value.trim_start_matches("0x")) {
                            Ok(b) => b,
                            Err(e) => return JsOutcome::err(format!("bad hex value: {e}")),
                        };
                        let mut g = inner.lock().await;
                        let Some(s) = g.services.get_mut(&svc) else {
                            return JsOutcome::err(format!("unknown service {svc}"));
                        };
                        let Some(c) = s.characteristics.get_mut(&chr) else {
                            return JsOutcome::err(format!("unknown characteristic {chr}"));
                        };
                        match c.write(&p, wor.0.unwrap_or(false), &bytes).await {
                            Ok(()) => JsOutcome::empty(),
                            Err(e) => JsOutcome::err(e.to_string()),
                        }
                    }
                },
            ),
        )?
    };

    // updateRssi() -- infallible
    let update_rssi = {
        let (p, inner) = (peripheral.clone(), inner.clone());
        Function::new(
            ctx.clone(),
            Async(move || {
                let (p, inner) = (p.clone(), inner.clone());
                async move {
                    inner.lock().await.update_rssi(&p).await;
                    JsOutcome::<String>::empty()
                }
            }),
        )?
    };

    let raw = Object::new(ctx.clone())?;
    raw.set("snapshot", snapshot)?;
    raw.set("connect", connect)?;
    raw.set("disconnect", disconnect)?;
    raw.set("enumerate", enumerate)?;
    raw.set("read", read)?;
    raw.set("subscribe", subscribe)?;
    raw.set("write", write)?;
    raw.set("updateRssi", update_rssi)?;

    let factory: Function = ctx.eval(
        r#"(id, raw) => {
            const u = ([json, err]) => {
                if (err) throw new Error(err);
                return json == null ? undefined : JSON.parse(json);
            };
            // Forward with spread so omitted args stay ABSENT (not `undefined`),
            // letting rquickjs `Opt<_>` see them as None.
            const m = (name) => (...args) => raw[name](...args).then(u);
            return {
                id,
                snapshot:   m("snapshot"),
                connect:    m("connect"),
                disconnect: m("disconnect"),
                enumerate:  m("enumerate"),
                read:       m("read"),
                subscribe:  m("subscribe"),
                write:      m("write"),
                updateRssi: m("updateRssi"),
            };
        }"#,
    )?;

    factory.call((id, raw))
}
