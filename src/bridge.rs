//! JavaScript bridge for the BLE scanner.
//!
//! Installs a global `scan(opts)` into a QuickJS (`rquickjs`) async context.
//! `scan` returns a *breakable async-iterable* of device objects; each device
//! object exposes the `DeviceInfo` operations as async methods and carries its
//! own `Peripheral` handle.
//!
//! ```js
//! for await (const dev of scan({ name: "INA219" })) {
//!     await dev.connect();
//!     await dev.enumerate();
//!     // read specific characteristics, optionally decoded, into a { uuid: value } dict:
//!     const vals = await dev.read_direct(["00000002-...::u16", "00000003-..."]);
//!     // raw bytes as ArrayBuffer instead of hex string:
//!     const raw = await dev.read_direct(["00000002-..."], true);
//!     // write accepts a hex string, an ArrayBuffer, or a Uint8Array:
//!     await dev.write("00000004-...", "01ff");
//!     await dev.write("00000004-...", new Uint8Array([1, 255]));
//!     // notifications (subscribe first); break to stop:
//!     await dev.subscribe();
//!     for await (const n of dev.notifications()) { console.log(n.uuid, n.value); break; }
//!     await dev.disconnect();
//!     break;
//! }
//! ```
//!
//! Binary value model (shared by read_direct / notifications / write):
//! * default: hex string
//! * `as_array_buffer = true`: an `ArrayBuffer` (read_direct / notifications)
//! * write accepts hex string | ArrayBuffer | Uint8Array (normalised in JS)
//!
//! Adjust the `use crate::...` paths below to match your module layout.

use std::pin::Pin;
use std::sync::Arc;

use btleplug::api::{Central, Peripheral as _, ValueNotification};
use btleplug::platform::{Adapter, Peripheral};
use futures::lock::Mutex;
use futures::{Stream, StreamExt};
use regex::Regex;
use rquickjs::{
    function::{Async, Opt},
    Array, Ctx, FromJs, Function, IntoJs, Object, Result as JsResult, TypedArray, Value,
};
use uuid::Uuid;

use crate::characteristic_data::CharFormat; // <-- adjust
use crate::scanner::DeviceScanner; // <-- adjust (module holding DeviceScanner)
use crate::types::DeviceInfo; // <-- adjust
use crate::util::{make_regex_filter, make_uuid_filter, parse_decoder, parse_uuid}; // <-- adjust

// ===========================================================================
// Result carrier: [value, error] (tuples don't implement IntoJs)
// ===========================================================================

struct JsOutcome<T> {
    value: Option<T>,
    error: Option<String>,
}

impl<T> JsOutcome<T> {
    fn ok(value: T) -> Self {
        Self {
            value: Some(value),
            error: None,
        }
    }
    fn empty() -> Self {
        Self {
            value: None,
            error: None,
        }
    }
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

/// Encode raw bytes for JS: an `ArrayBuffer` if `as_array_buffer`, else a hex string.
fn bytes_to_js<'js>(ctx: &Ctx<'js>, bytes: Vec<u8>, as_array_buffer: bool) -> JsResult<Value<'js>> {
    if as_array_buffer {
        Ok(TypedArray::<u8>::new(ctx.clone(), bytes)?.arraybuffer()?.into_value())
    } else {
        hex::encode(&bytes).into_js(ctx)
    }
}

// ===========================================================================
// Public entry point
// ===========================================================================

/// Install a global `scan(opts)` function into `ctx`.
///
/// `opts` (all fields optional):
/// `{ rssi?, name?|names?, device?|devices?, filter_seen? }`.
pub fn install_scan<'js>(ctx: &Ctx<'js>, central: Adapter) -> JsResult<()> {
    let scan = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, opts: ScanOpts| -> JsResult<Object<'js>> {
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
        let Some(o) = value.into_object() else {
            return Ok(ScanOpts {
                rssi: None,
                names: Vec::new(),
                devices: Vec::new(),
                filter_seen: false,
            });
        };
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
// Generic breakable async-iterable factory (shared by scanner + notifications)
// ===========================================================================

fn make_async_iterable<'js>(
    ctx: &Ctx<'js>,
    rust_next: Function<'js>,
    rust_return: Function<'js>,
) -> JsResult<Object<'js>> {
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
// Scanner iterable
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

    let n_central = central.clone();
    let n_state = state.clone();
    let rust_next = Function::new(
        ctx.clone(),
        Async(move || {
            let central = n_central.clone();
            let state = n_state.clone();
            async move {
                let mut guard = state.lock().await;
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
                        Err(e) => return JsOutcome::err(e.to_string()),
                    }
                }
                match &mut *guard {
                    ScanState::Running(s) => match s.next_match().await {
                        Ok(Some((peripheral, info))) => JsOutcome::ok(ScannedDevice { peripheral, info }),
                        Ok(None) => JsOutcome::empty(),
                        Err(e) => JsOutcome::err(e.to_string()),
                    },
                    _ => JsOutcome::empty(),
                }
            }
        }),
    )?;

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
                None::<String>
            }
        }),
    )?;

    make_async_iterable(ctx, rust_next, rust_return)
}

// ===========================================================================
// Notifications iterable
// ===========================================================================

enum NotifyState {
    Idle,
    Running(Pin<Box<dyn Stream<Item = ValueNotification> + Send>>),
    Closed,
}

fn build_notifications_iterable<'js>(
    ctx: &Ctx<'js>,
    peripheral: Peripheral,
    as_array_buffer: bool,
) -> JsResult<Object<'js>> {
    let state = Arc::new(Mutex::new(NotifyState::Idle));

    let n_p = peripheral.clone();
    let n_state = state.clone();
    let rust_next = Function::new(
        ctx.clone(),
        Async(move || {
            let p = n_p.clone();
            let state = n_state.clone();
            async move {
                let mut guard = state.lock().await;
                if matches!(&*guard, NotifyState::Idle) {
                    match p.notifications().await {
                        Ok(s) => *guard = NotifyState::Running(s),
                        Err(e) => {
                            *guard = NotifyState::Closed;
                            return JsOutcome::err(e.to_string());
                        }
                    }
                }
                match &mut *guard {
                    NotifyState::Running(s) => match s.next().await {
                        Some(n) => JsOutcome::ok(Notification {
                            uuid: n.uuid,
                            value: n.value,
                            as_array_buffer,
                        }),
                        None => JsOutcome::empty(), // stream ended
                    },
                    _ => JsOutcome::empty(),
                }
            }
        }),
    )?;

    let r_state = state.clone();
    let rust_return = Function::new(
        ctx.clone(),
        Async(move || {
            let state = r_state.clone();
            async move {
                // Dropping the stream stops delivery to this consumer.
                *state.lock().await = NotifyState::Closed;
                None::<String>
            }
        }),
    )?;

    make_async_iterable(ctx, rust_next, rust_return)
}

/// A single notification; `IntoJs` -> `{ uuid, value }` (value hex or ArrayBuffer).
struct Notification {
    uuid: Uuid,
    value: Vec<u8>,
    as_array_buffer: bool,
}

impl<'js> IntoJs<'js> for Notification {
    fn into_js(self, ctx: &Ctx<'js>) -> JsResult<Value<'js>> {
        let obj = Object::new(ctx.clone())?;
        obj.set("uuid", self.uuid.to_string())?;
        obj.set("value", bytes_to_js(ctx, self.value, self.as_array_buffer)?)?;
        Ok(obj.into_value())
    }
}

// ===========================================================================
// Device object
// ===========================================================================

/// Yielded by the iterator; `IntoJs` builds the device object at resolve time.
struct ScannedDevice {
    peripheral: Peripheral,
    info: DeviceInfo,
}

impl<'js> IntoJs<'js> for ScannedDevice {
    fn into_js(self, ctx: &Ctx<'js>) -> JsResult<Value<'js>> {
        Ok(device_into_js(ctx, self.peripheral, self.info)?.into_value())
    }
}

/// `"uuid"` or `"uuid::fmt"` -> (Uuid, optional decode format).
fn parse_read_spec(items: &[String]) -> Result<Vec<(Uuid, Option<CharFormat>)>, String> {
    items
        .iter()
        .map(|s| match s.split_once("::") {
            Some((u, fmt)) => {
                let uuid = parse_uuid(u).map_err(|e| format!("bad uuid `{u}`: {e}"))?;
                let fmt = CharFormat::try_from(fmt).map_err(|e| format!("bad format `{fmt}`: {e}"))?;
                Ok((uuid, Some(fmt)))
            }
            None => {
                let uuid = parse_uuid(s).map_err(|e| format!("bad uuid `{s}`: {e}"))?;
                Ok((uuid, None))
            }
        })
        .collect()
}

enum ReadValue {
    Decoded(serde_json::Value),
    Raw(Vec<u8>),
}

/// Result of read_direct; `IntoJs` -> `{ uuid: value }` object.
struct ReadResults {
    items: Vec<(Uuid, ReadValue)>,
    as_array_buffer: bool,
}

impl<'js> IntoJs<'js> for ReadResults {
    fn into_js(self, ctx: &Ctx<'js>) -> JsResult<Value<'js>> {
        let obj = Object::new(ctx.clone())?;
        for (uuid, val) in self.items {
            let jsval = match val {
                ReadValue::Decoded(json) => {
                    let s = serde_json::to_string(&json).unwrap_or_else(|_| "null".to_string());
                    ctx.json_parse(s)?
                }
                ReadValue::Raw(bytes) => bytes_to_js(ctx, bytes, self.as_array_buffer)?,
            };
            obj.set(uuid.to_string(), jsval)?;
        }
        Ok(obj.into_value())
    }
}

/// Build a JS object wrapping a `DeviceInfo` + its `Peripheral`.
fn device_into_js<'js>(ctx: &Ctx<'js>, peripheral: Peripheral, info: DeviceInfo) -> JsResult<Object<'js>> {
    let id = info.id.clone();
    let inner = Arc::new(Mutex::new(info));

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

    // read(decode?) -- reads all enumerated characteristics into DeviceInfo
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

    // read_direct(chars, as_array_buffer?) -> { uuid: value }
    let read_direct = {
        let (p, inner) = (peripheral.clone(), inner.clone());
        Function::new(
            ctx.clone(),
            Async(move |chars: Opt<Vec<String>>, as_array_buffer: Opt<bool>| {
                let (p, inner) = (p.clone(), inner.clone());
                async move {
                    let specs = match parse_read_spec(&chars.0.unwrap_or_default()) {
                        Ok(s) => s,
                        Err(e) => return JsOutcome::<ReadResults>::err(e),
                    };
                    let as_ab = as_array_buffer.0.unwrap_or(false);
                    // Hold the device lock to serialise peripheral access.
                    let _guard = inner.lock().await;
                    let mut available = p.characteristics();
                    if available.is_empty() {
                        if let Err(e) = p.discover_services().await {
                            return JsOutcome::err(format!("discover failed: {e}"));
                        }
                        available = p.characteristics();
                    }
                    let mut items = Vec::with_capacity(specs.len());
                    for (uuid, fmt) in specs {
                        let Some(ch) = available.iter().find(|c| c.uuid == uuid) else {
                            return JsOutcome::err(format!("characteristic {uuid} not found (enumerate first?)"));
                        };
                        let bytes = match p.read(ch).await {
                            Ok(b) => b,
                            Err(e) => return JsOutcome::err(format!("read {uuid} failed: {e}")),
                        };
                        let val = match fmt {
                            // Fall back to raw bytes if the decode fails.
                            Some(f) => match f.decode_value(&bytes) {
                                Ok(v) => ReadValue::Decoded(v),
                                Err(_) => ReadValue::Raw(bytes),
                            },
                            None => ReadValue::Raw(bytes),
                        };
                        items.push((uuid, val));
                    }
                    JsOutcome::ok(ReadResults {
                        items,
                        as_array_buffer: as_ab,
                    })
                }
            }),
        )?
    };

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
    // (the JS shim normalises ArrayBuffer/Uint8Array -> hex before calling this)
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

    // notifications(as_array_buffer?) -> breakable async-iterable (returns sync, like scan)
    let notifications = {
        let peripheral = peripheral.clone();
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, as_array_buffer: Opt<bool>| -> JsResult<Object<'js>> {
                build_notifications_iterable(&ctx, peripheral.clone(), as_array_buffer.0.unwrap_or(false))
            },
        )?
    };

    let raw = Object::new(ctx.clone())?;
    raw.set("snapshot", snapshot)?;
    raw.set("connect", connect)?;
    raw.set("disconnect", disconnect)?;
    raw.set("enumerate", enumerate)?;
    raw.set("read", read)?;
    raw.set("read_direct", read_direct)?;
    raw.set("subscribe", subscribe)?;
    raw.set("write", write)?;
    raw.set("updateRssi", update_rssi)?;
    raw.set("notifications", notifications)?;

    let factory: Function = ctx.eval(
        r#"(id, raw) => {
            // unwrap [json, err] -> JSON.parse(value)
            const u = ([json, err]) => {
                if (err) throw new Error(err);
                return json == null ? undefined : JSON.parse(json);
            };
            // unwrap [value, err] -> value as-is (real JS objects / ArrayBuffers)
            const u2 = ([val, err]) => {
                if (err) throw new Error(err);
                return val;
            };
            const m = (name) => (...args) => raw[name](...args).then(u);
            const m2 = (name) => (...args) => raw[name](...args).then(u2);
            // hex | ArrayBuffer | TypedArray -> hex string for write()
            const toHex = (v) => {
                if (typeof v === "string") return v;
                const bytes = v instanceof ArrayBuffer ? new Uint8Array(v)
                            : ArrayBuffer.isView(v)     ? new Uint8Array(v.buffer, v.byteOffset, v.byteLength)
                            : new Uint8Array(v);
                let s = "";
                for (const b of bytes) s += b.toString(16).padStart(2, "0");
                return s;
            };
            return {
                id,
                snapshot:    m("snapshot"),
                connect:     m("connect"),
                disconnect:  m("disconnect"),
                enumerate:   m("enumerate"),
                read:        m("read"),
                read_direct: m2("read_direct"),
                subscribe:   m("subscribe"),
                write:       (s, c, v, w) => raw.write(s, c, toHex(v), w).then(u),
                updateRssi:  m("updateRssi"),
                // returns an async-iterable synchronously (like scan)
                notifications: (...args) => raw.notifications(...args),
                // callback-based notifications. Returns a handle: { stop() }.
                // `cb` is invoked with each { uuid, value }; callback errors are
                // swallowed so one bad call doesn't tear down the subscription.
                on_notification(cb, asArrayBuffer) {
                    if (typeof cb !== "function") {
                        throw new Error("on_notification requires a callback function");
                    }
                    const it = this.notifications(asArrayBuffer ?? false);
                    let stopped = false;
                    (async () => {
                        try {
                            for await (const n of it) {
                                if (stopped) break;
                                try { cb(n); } catch (_) { /* ignore callback errors */ }
                            }
                        } catch (_) { /* iterable error / closed */ }
                    })();
                    return { stop: () => { stopped = true; return it.close(); } };
                },
            };
        }"#,
    )?;

    factory.call((id, raw))
}
