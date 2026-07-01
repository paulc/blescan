//! JavaScript bridge for the BLE scanner.
//!
//! Installs a global `scan(opts)` into a QuickJS (`rquickjs`) async context.
//! `scan` returns a *breakable async-iterable* of device objects; each device
//! object exposes direct, by-UUID operations and carries its own `Peripheral`.
//!
//! ```js
//! for await (const dev of scan({ name: "INA219" })) {
//!     await dev.connect();
//!     await dev.enumerate();
//!
//!     // read specific characteristics into a { uuid: value } dict.
//!     // "uuid::fmt" decodes to an array of fields; "uuid" returns hex
//!     // (or ArrayBuffer with the flag).
//!     const vals = await dev.read(["00000002-...::u16", "00000003-..."]);
//!     const raw  = await dev.read(["00000002-..."], true);   // { uuid: ArrayBuffer }
//!
//!     // write(spec, value, withoutResponse?). "uuid::fmt" encodes the JS value
//!     // via CharFormat (fmt is a comma-list of field types); a lone scalar is
//!     // wrapped into a 1-element array. Plain "uuid" takes hex / binary.
//!     await dev.write("00000004-...::u32", 1);
//!     await dev.write("00000005-...::u8,u16", [1, 2]);
//!     await dev.write("00000004-...", "01ff");            // hex
//!     await dev.write("00000004-...", new Uint8Array([1, 255])); // binary
//!
//!     // subscribe to specific characteristics; "::fmt" decodes their notifications.
//!     await dev.subscribe(["00000003-...::u16"]);
//!     for await (const n of dev.notifications()) { console.log(n.uuid, n.value); break; }
//!     // or callback style:
//!     const sub = dev.on_notification((n) => console.log(n.uuid, n.value));
//!     // sub.stop();
//!
//!     await dev.disconnect();
//!     break;
//! }
//! ```
//!
//! Value model (read / notifications): a registered `::fmt` runs the bytes
//! through `CharFormat::decode`, which always yields a JSON *array* of the
//! decoded fields (so `::u16` -> `[v]`, `::u8,u16` -> `[a, b]`); without a fmt
//! the value is a hex string, or an `ArrayBuffer` when `as_array_buffer` is set.
//! `write` mirrors this: a `::fmt` runs the value array through
//! `CharFormat::encode_value`, otherwise it takes a hex string | ArrayBuffer |
//! Uint8Array (normalised in JS).
//!
//! Adjust the `use crate::...` paths below to match your module layout.

use std::collections::{BTreeSet, HashMap};
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex};

use btleplug::api::{Central, Characteristic, Peripheral as _, ValueNotification, WriteType};
use btleplug::platform::{Adapter, Peripheral};
use futures::channel::oneshot;
use futures::future::{select, Either, Shared};
use futures::lock::Mutex;
use futures::{FutureExt, Stream, StreamExt};
use regex::Regex;
use rquickjs::{
    function::{Async, Opt},
    Array, Ctx, FromJs, Function, IntoJs, Object, Result as JsResult, TypedArray, Value,
};
use uuid::Uuid;

use crate::characteristic_data::CharFormat; // <-- adjust
use crate::scanner::DeviceScanner; // <-- adjust (module holding DeviceScanner)
use crate::types::DeviceInfo; // <-- adjust
use crate::util::{make_regex_filter, make_uuid_filter, parse_uuid}; // <-- adjust

/// Decode formats registered by `subscribe`, consulted when notifications arrive.
type NotifyDecode = Arc<StdMutex<HashMap<Uuid, CharFormat>>>;

/// One-shot cancel for a notifications iterator. Either the iterator's own
/// `return()`/`close()` or a device-level `unsubscribe` can fire it; whoever
/// fires first takes the sender. `next()` selects on the (shared) receiver.
type CancelHandle = Arc<StdMutex<Option<oneshot::Sender<()>>>>;
/// Registry of cancel handles for every live notifications iterator on a device.
type CancelRegistry = Arc<StdMutex<Vec<CancelHandle>>>;

fn fire_cancel(handle: &CancelHandle) {
    if let Some(tx) = handle.lock().unwrap().take() {
        let _ = tx.send(());
    }
}

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

// ===========================================================================
// Characteristic value representation (shared by read + notifications)
// ===========================================================================

enum CharValue {
    Decoded(serde_json::Value),
    Raw(Vec<u8>),
}

/// Decoded -> JS value; Raw -> hex string, or ArrayBuffer if `as_array_buffer`.
fn char_value_to_js<'js>(ctx: &Ctx<'js>, val: CharValue, as_array_buffer: bool) -> JsResult<Value<'js>> {
    match val {
        CharValue::Decoded(json) => {
            let s = serde_json::to_string(&json).unwrap_or_else(|_| "null".to_string());
            ctx.json_parse(s)
        }
        CharValue::Raw(bytes) => {
            if as_array_buffer {
                Ok(TypedArray::<u8>::new(ctx.clone(), bytes)?.arraybuffer()?.into_value())
            } else {
                hex::encode(&bytes).into_js(ctx)
            }
        }
    }
}

/// `"uuid"` or `"uuid::fmt"` -> (Uuid, optional `CharFormat` for encode/decode).
fn parse_spec(s: &str) -> Result<(Uuid, Option<CharFormat>), String> {
    match s.split_once("::") {
        Some((u, fmt)) => {
            let uuid = parse_uuid(u).map_err(|e| format!("bad uuid `{u}`: {e}"))?;
            let fmt = CharFormat::try_from(fmt).map_err(|e| format!("bad format `{fmt}`: {e}"))?;
            Ok((uuid, Some(fmt)))
        }
        None => {
            let uuid = parse_uuid(s).map_err(|e| format!("bad uuid `{s}`: {e}"))?;
            Ok((uuid, None))
        }
    }
}

/// Parse a list of specs (for read / subscribe).
fn parse_specs(items: &[String]) -> Result<Vec<(Uuid, Option<CharFormat>)>, String> {
    items.iter().map(|s| parse_spec(s)).collect()
}

/// Discover services if needed, then return the peripheral's characteristics.
async fn ensure_characteristics(p: &Peripheral) -> Result<BTreeSet<Characteristic>, String> {
    let mut available = p.characteristics();
    if available.is_empty() {
        p.discover_services()
            .await
            .map_err(|e| format!("discover failed: {e}"))?;
        available = p.characteristics();
    }
    Ok(available)
}

/// Build the byte payload for a write. With a `CharFormat` the JSON `value` (an
/// array of field values) is encoded via `CharFormat::encode_value`; without one
/// the string is treated as hex (the JS shim has normalised binary inputs to hex).
fn write_payload(fmt: Option<&CharFormat>, value: &str) -> Result<Vec<u8>, String> {
    match fmt {
        Some(cf) => {
            let jv: serde_json::Value = serde_json::from_str(value).map_err(|e| format!("bad value JSON: {e}"))?;
            let data = cf.encode_value(&jv).map_err(|e| format!("encode failed: {e}"))?;
            Ok(data.as_slice().to_vec())
        }
        None => hex::decode(value.trim_start_matches("0x")).map_err(|e| format!("bad hex value: {e}")),
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
    decode: NotifyDecode,
    cancel: CancelHandle,
    cancel_rx: Shared<oneshot::Receiver<()>>,
    as_array_buffer: bool,
) -> JsResult<Object<'js>> {
    let state = Arc::new(Mutex::new(NotifyState::Idle));

    // Local outcome of one pull, computed while the state lock is held so we can
    // mutate the state afterwards without overlapping borrows.
    enum Step {
        Item(ValueNotification),
        Done,
        Cancelled,
    }

    let n_p = peripheral.clone();
    let n_state = state.clone();
    let n_decode = decode.clone();
    let n_cancel = cancel_rx.clone();
    let rust_next = Function::new(
        ctx.clone(),
        Async(move || {
            let p = n_p.clone();
            let state = n_state.clone();
            let decode = n_decode.clone();
            let cancel = n_cancel.clone();
            async move {
                let mut guard = state.lock().await;
                if matches!(&*guard, NotifyState::Idle) {
                    // Already cancelled before the stream even opened?
                    if cancel.clone().now_or_never().is_some() {
                        *guard = NotifyState::Closed;
                        return JsOutcome::empty();
                    }
                    match p.notifications().await {
                        Ok(s) => *guard = NotifyState::Running(s),
                        Err(e) => {
                            *guard = NotifyState::Closed;
                            return JsOutcome::err(e.to_string());
                        }
                    }
                }
                let step = match &mut *guard {
                    NotifyState::Running(s) => {
                        // Wait for the next notification OR a cancel, whichever
                        // comes first. Cancel never touches the state lock, so it
                        // wakes a parked pull immediately.
                        match select(cancel.clone(), s.next()).await {
                            Either::Left(_) => Step::Cancelled,
                            Either::Right((Some(n), _)) => Step::Item(n),
                            Either::Right((None, _)) => Step::Done,
                        }
                    }
                    _ => Step::Done,
                };
                match step {
                    Step::Item(n) => {
                        // Decode if a format was registered at subscribe time.
                        let decoded = {
                            let map = decode.lock().unwrap();
                            map.get(&n.uuid).and_then(|f| f.decode(&n.value).ok())
                        };
                        let value = match decoded {
                            Some(v) => CharValue::Decoded(v),
                            None => CharValue::Raw(n.value),
                        };
                        JsOutcome::ok(Notification {
                            uuid: n.uuid,
                            value,
                            as_array_buffer,
                        })
                    }
                    Step::Done | Step::Cancelled => {
                        *guard = NotifyState::Closed; // drops the stream
                        JsOutcome::empty()
                    }
                }
            }
        }),
    )?;

    let r_state = state.clone();
    let r_cancel = cancel.clone();
    let rust_return = Function::new(
        ctx.clone(),
        Async(move || {
            let state = r_state.clone();
            let cancel = r_cancel.clone();
            async move {
                fire_cancel(&cancel); // wake a parked pull promptly
                *state.lock().await = NotifyState::Closed; // drops the stream
                None::<String>
            }
        }),
    )?;

    make_async_iterable(ctx, rust_next, rust_return)
}

/// A single notification; `IntoJs` -> `{ uuid, value }`.
struct Notification {
    uuid: Uuid,
    value: CharValue,
    as_array_buffer: bool,
}

impl<'js> IntoJs<'js> for Notification {
    fn into_js(self, ctx: &Ctx<'js>) -> JsResult<Value<'js>> {
        let obj = Object::new(ctx.clone())?;
        obj.set("uuid", self.uuid.to_string())?;
        obj.set("value", char_value_to_js(ctx, self.value, self.as_array_buffer)?)?;
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

/// Result of `read`; `IntoJs` -> `{ uuid: value }` object.
struct ReadResults {
    items: Vec<(Uuid, CharValue)>,
    as_array_buffer: bool,
}

impl<'js> IntoJs<'js> for ReadResults {
    fn into_js(self, ctx: &Ctx<'js>) -> JsResult<Value<'js>> {
        let obj = Object::new(ctx.clone())?;
        for (uuid, val) in self.items {
            obj.set(uuid.to_string(), char_value_to_js(ctx, val, self.as_array_buffer)?)?;
        }
        Ok(obj.into_value())
    }
}

/// Build a JS object wrapping a `DeviceInfo` + its `Peripheral`.
fn device_into_js<'js>(ctx: &Ctx<'js>, peripheral: Peripheral, info: DeviceInfo) -> JsResult<Object<'js>> {
    let id = info.id.clone();
    let inner = Arc::new(Mutex::new(info));
    // Decode formats registered by subscribe(), used when notifications arrive.
    let notify_decode: NotifyDecode = Arc::new(StdMutex::new(HashMap::new()));
    // Cancel handles for every live notifications iterator, so unsubscribe()
    // can close them.
    let notif_cancels: CancelRegistry = Arc::new(StdMutex::new(Vec::new()));

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

    // read(chars, as_array_buffer?) -> { uuid: value }
    // chars: ["uuid"] or ["uuid::fmt"]; fmt decodes, otherwise hex / ArrayBuffer.
    let read = {
        let (p, inner) = (peripheral.clone(), inner.clone());
        Function::new(
            ctx.clone(),
            Async(move |chars: Opt<Vec<String>>, as_array_buffer: Opt<bool>| {
                let (p, inner) = (p.clone(), inner.clone());
                async move {
                    let specs = match parse_specs(&chars.0.unwrap_or_default()) {
                        Ok(s) => s,
                        Err(e) => return JsOutcome::<ReadResults>::err(e),
                    };
                    let as_ab = as_array_buffer.0.unwrap_or(false);
                    let _guard = inner.lock().await; // serialise peripheral access
                    let available = match ensure_characteristics(&p).await {
                        Ok(a) => a,
                        Err(e) => return JsOutcome::err(e),
                    };
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
                            Some(f) => match f.decode(&bytes) {
                                Ok(v) => CharValue::Decoded(v),
                                Err(_) => CharValue::Raw(bytes), // fall back to raw
                            },
                            None => CharValue::Raw(bytes),
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

    // subscribe(chars) -> [uuid...]
    // chars: ["uuid"] or ["uuid::fmt"]; a fmt registers a decoder for that
    // characteristic's notifications.
    let subscribe = {
        let (p, inner, decode) = (peripheral.clone(), inner.clone(), notify_decode.clone());
        Function::new(
            ctx.clone(),
            Async(move |chars: Opt<Vec<String>>| {
                let (p, inner, decode) = (p.clone(), inner.clone(), decode.clone());
                async move {
                    let specs = match parse_specs(&chars.0.unwrap_or_default()) {
                        Ok(s) => s,
                        Err(e) => return JsOutcome::<String>::err(e),
                    };
                    let _guard = inner.lock().await;
                    let available = match ensure_characteristics(&p).await {
                        Ok(a) => a,
                        Err(e) => return JsOutcome::err(e),
                    };
                    let mut subscribed = Vec::with_capacity(specs.len());
                    for (uuid, fmt) in specs {
                        let Some(ch) = available.iter().find(|c| c.uuid == uuid) else {
                            return JsOutcome::err(format!("characteristic {uuid} not found (enumerate first?)"));
                        };
                        if let Err(e) = p.subscribe(ch).await {
                            return JsOutcome::err(format!("subscribe {uuid} failed: {e}"));
                        }
                        if let Some(f) = fmt {
                            decode.lock().unwrap().insert(uuid, f);
                        }
                        subscribed.push(uuid.to_string());
                    }
                    match serde_json::to_string(&subscribed) {
                        Ok(j) => JsOutcome::ok(j),
                        Err(e) => JsOutcome::err(e.to_string()),
                    }
                }
            }),
        )?
    };

    // unsubscribe(chars) -> [uuid...]
    // chars: ["uuid"] or ["uuid::fmt"] (fmt ignored); stops notifications for
    // each characteristic and drops any decoder registered by subscribe().
    let unsubscribe = {
        let (p, inner, decode, cancels) = (
            peripheral.clone(),
            inner.clone(),
            notify_decode.clone(),
            notif_cancels.clone(),
        );
        Function::new(
            ctx.clone(),
            Async(move |chars: Opt<Vec<String>>| {
                let (p, inner, decode, cancels) = (p.clone(), inner.clone(), decode.clone(), cancels.clone());
                async move {
                    let specs = match parse_specs(&chars.0.unwrap_or_default()) {
                        Ok(s) => s,
                        Err(e) => return JsOutcome::<String>::err(e),
                    };
                    let _guard = inner.lock().await;
                    let available = match ensure_characteristics(&p).await {
                        Ok(a) => a,
                        Err(e) => return JsOutcome::err(e),
                    };
                    let mut unsubscribed = Vec::with_capacity(specs.len());
                    for (uuid, _fmt) in specs {
                        let Some(ch) = available.iter().find(|c| c.uuid == uuid) else {
                            return JsOutcome::err(format!("characteristic {uuid} not found (enumerate first?)"));
                        };
                        if let Err(e) = p.unsubscribe(ch).await {
                            return JsOutcome::err(format!("unsubscribe {uuid} failed: {e}"));
                        }
                        decode.lock().unwrap().remove(&uuid);
                        unsubscribed.push(uuid.to_string());
                    }
                    // Close every live notifications iterator on this device. The
                    // underlying btleplug stream is device-wide (not per-characteristic),
                    // so there's no finer granularity to target.
                    for handle in cancels.lock().unwrap().drain(..) {
                        fire_cancel(&handle);
                    }
                    match serde_json::to_string(&unsubscribed) {
                        Ok(j) => JsOutcome::ok(j),
                        Err(e) => JsOutcome::err(e.to_string()),
                    }
                }
            }),
        )?
    };

    // write(spec, value, withoutResponse?)
    //   spec : "uuid" or "uuid::fmt"
    //   value: with "::fmt", a JS value (scalar) or array,
    //          passed through as JSON; without a fmt, the JS shim has already
    //          normalised a hex string / ArrayBuffer / Uint8Array to hex.
    let write = {
        let (p, inner) = (peripheral.clone(), inner.clone());
        Function::new(
            ctx.clone(),
            Async(move |spec: String, value: String, wor: Opt<bool>| {
                let (p, inner) = (p.clone(), inner.clone());
                async move {
                    let (uuid, fmt) = match parse_spec(&spec) {
                        Ok(s) => s,
                        Err(e) => return JsOutcome::<String>::err(e),
                    };
                    let bytes = match write_payload(fmt.as_ref(), &value) {
                        Ok(b) => b,
                        Err(e) => return JsOutcome::err(e),
                    };
                    let write_type = if wor.0.unwrap_or(false) {
                        WriteType::WithoutResponse
                    } else {
                        WriteType::WithResponse
                    };
                    let _guard = inner.lock().await; // serialise peripheral access
                    let available = match ensure_characteristics(&p).await {
                        Ok(a) => a,
                        Err(e) => return JsOutcome::err(e),
                    };
                    let Some(ch) = available.iter().find(|c| c.uuid == uuid) else {
                        return JsOutcome::err(format!("characteristic {uuid} not found (enumerate first?)"));
                    };
                    match p.write(ch, &bytes, write_type).await {
                        Ok(()) => JsOutcome::empty(),
                        Err(e) => JsOutcome::err(format!("write {uuid} failed: {e}")),
                    }
                }
            }),
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

    // notifications(as_array_buffer?) -> breakable async-iterable (returns sync)
    let notifications = {
        let (peripheral, decode, cancels) = (peripheral.clone(), notify_decode.clone(), notif_cancels.clone());
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, as_array_buffer: Opt<bool>| -> JsResult<Object<'js>> {
                let (tx, rx) = oneshot::channel::<()>();
                let handle: CancelHandle = Arc::new(StdMutex::new(Some(tx)));
                {
                    let mut g = cancels.lock().unwrap();
                    g.retain(|h| h.lock().unwrap().is_some()); // drop spent handles
                    g.push(handle.clone());
                }
                build_notifications_iterable(
                    &ctx,
                    peripheral.clone(),
                    decode.clone(),
                    handle,
                    rx.shared(),
                    as_array_buffer.0.unwrap_or(false),
                )
            },
        )?
    };

    let raw = Object::new(ctx.clone())?;
    raw.set("snapshot", snapshot)?;
    raw.set("connect", connect)?;
    raw.set("disconnect", disconnect)?;
    raw.set("enumerate", enumerate)?;
    raw.set("read", read)?;
    raw.set("subscribe", subscribe)?;
    raw.set("unsubscribe", unsubscribe)?;
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
                snapshot:   m("snapshot"),
                connect:    m("connect"),
                disconnect: m("disconnect"),
                enumerate:  m("enumerate"),
                read: (spec, ...args) => raw.read(Array.isArray(spec) ? spec : [spec], ...args).then(u2),
                subscribe:  (spec, ...args) => raw.subscribe(Array.isArray(spec) ? spec : [spec], ...args).then(u),
                unsubscribe: (spec, ...args) => raw.unsubscribe(Array.isArray(spec) ? spec : [spec], ...args).then(u),
                // write(spec, value, withoutResponse?). With "::fmt" the value is
                // sent as JSON for CharFormat::encode_value, which expects an array
                // of field values; a lone scalar is wrapped so write("x::u32", 1)
                // works as well as write("x::u8,u16", [1, 2]). Without a fmt the
                // value is normalised to hex (hex string | ArrayBuffer | Uint8Array).
                write: (spec, value, withoutResponse) =>
                    raw.write(
                        spec,
                        spec.includes("::")
                            ? JSON.stringify(Array.isArray(value) ? value : [value])
                            : toHex(value),
                        withoutResponse ?? false,
                    ).then(u),
                updateRssi: m("updateRssi"),
                // returns an async-iterable synchronously (like scan)
                notifications: (...args) => raw.notifications(...args),
                // callback-based notifications. Returns a handle: { stop() }.
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
