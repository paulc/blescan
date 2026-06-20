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
//!     // "uuid::fmt" decodes; "uuid" returns hex (or ArrayBuffer with the flag).
//!     const vals = await dev.read(["00000002-...::u16", "00000003-..."]);
//!     const raw  = await dev.read(["00000002-..."], true);   // { uuid: ArrayBuffer }
//!
//!     // write(spec, value, withoutResponse?). "uuid::fmt" encodes the JS value
//!     // (scalar, or an array for struct<...>); plain "uuid" takes hex / binary.
//!     await dev.write("00000004-...::u32", 1);
//!     await dev.write("00000005-...::struct<u32,u32>", [1, 2]);
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
//! Value model (read / notifications): a registered `::fmt` yields the decoded
//! value; otherwise a hex string, or an `ArrayBuffer` when `as_array_buffer` is
//! set. `write` mirrors this: a `::fmt` encodes the JS value (little-endian),
//! otherwise it takes a hex string | ArrayBuffer | Uint8Array (normalised in JS).
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
        Self { value: Some(value), error: None }
    }
    fn empty() -> Self {
        Self { value: None, error: None }
    }
    fn err(msg: String) -> Self {
        Self { value: None, error: Some(msg) }
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
fn char_value_to_js<'js>(
    ctx: &Ctx<'js>,
    val: CharValue,
    as_array_buffer: bool,
) -> JsResult<Value<'js>> {
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

/// `"uuid"` or `"uuid::fmt"` -> (Uuid, optional decode format).
fn parse_specs(items: &[String]) -> Result<Vec<(Uuid, Option<CharFormat>)>, String> {
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

/// Discover services if needed, then return the peripheral's characteristics.
async fn ensure_characteristics(p: &Peripheral) -> Result<BTreeSet<Characteristic>, String> {
    let mut available = p.characteristics();
    if available.is_empty() {
        p.discover_services().await.map_err(|e| format!("discover failed: {e}"))?;
        available = p.characteristics();
    }
    Ok(available)
}

/// `"uuid"` or `"uuid::fmt"` -> (Uuid, raw format string). Unlike `parse_specs`
/// the format is left un-parsed (write encoding uses `WriteFormat`, not `CharFormat`).
fn parse_uuid_spec(s: &str) -> Result<(Uuid, Option<String>), String> {
    match s.split_once("::") {
        Some((u, fmt)) => Ok((
            parse_uuid(u).map_err(|e| format!("bad uuid `{u}`: {e}"))?,
            Some(fmt.to_string()),
        )),
        None => Ok((parse_uuid(s).map_err(|e| format!("bad uuid `{s}`: {e}"))?, None)),
    }
}

// ---------------------------------------------------------------------------
// Write encoding. Self-contained (CharFormat only decodes). Little-endian, to
// match the BLE convention used on the read side. Grammar:
//   u8 u16 u32 u64 | i8 i16 i32 i64 | f32 f64 | struct<F, F, ...>
// A scalar format takes a single JS value; `struct<...>` takes an array.
// ---------------------------------------------------------------------------

enum WriteFormat {
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    F32,
    F64,
    Struct(Vec<WriteFormat>),
}

/// Split on top-level commas, respecting nested `<...>`.
fn split_top_level(s: &str) -> Result<Vec<&str>, String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth < 0 {
                    return Err("unbalanced '>' in format".into());
                }
            }
            ',' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err("unbalanced '<' in format".into());
    }
    parts.push(&s[start..]);
    Ok(parts)
}

fn parse_write_format(s: &str) -> Result<WriteFormat, String> {
    let s = s.trim();
    if let Some(inner) = s.strip_prefix("struct<").and_then(|x| x.strip_suffix('>')) {
        let fields = split_top_level(inner)?
            .into_iter()
            .map(|f| parse_write_format(f.trim()))
            .collect::<Result<Vec<_>, _>>()?;
        if fields.is_empty() {
            return Err("struct<> needs at least one field".into());
        }
        return Ok(WriteFormat::Struct(fields));
    }
    Ok(match s {
        "u8" => WriteFormat::U8,
        "u16" => WriteFormat::U16,
        "u32" => WriteFormat::U32,
        "u64" => WriteFormat::U64,
        "i8" => WriteFormat::I8,
        "i16" => WriteFormat::I16,
        "i32" => WriteFormat::I32,
        "i64" => WriteFormat::I64,
        "f32" => WriteFormat::F32,
        "f64" => WriteFormat::F64,
        other => {
            return Err(format!(
                "unsupported write format `{other}` \
                 (expected u8/u16/u32/u64, i8/i16/i32/i64, f32/f64, or struct<...>)"
            ))
        }
    })
}

fn json_int(v: &serde_json::Value) -> Result<i128, String> {
    if let Some(b) = v.as_bool() {
        return Ok(b as i128);
    }
    if let Some(u) = v.as_u64() {
        return Ok(u as i128);
    }
    if let Some(i) = v.as_i64() {
        return Ok(i as i128);
    }
    Err(format!("expected an integer, got `{v}`"))
}

fn json_f64(v: &serde_json::Value) -> Result<f64, String> {
    v.as_f64().ok_or_else(|| format!("expected a number, got `{v}`"))
}

fn encode_write_value(
    fmt: &WriteFormat,
    v: &serde_json::Value,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    macro_rules! put_int {
        ($t:ty) => {{
            let n = json_int(v)?;
            let x: $t = <$t>::try_from(n)
                .map_err(|_| format!("value {n} out of range for {}", stringify!($t)))?;
            out.extend_from_slice(&x.to_le_bytes());
        }};
    }
    match fmt {
        WriteFormat::U8 => put_int!(u8),
        WriteFormat::U16 => put_int!(u16),
        WriteFormat::U32 => put_int!(u32),
        WriteFormat::U64 => put_int!(u64),
        WriteFormat::I8 => put_int!(i8),
        WriteFormat::I16 => put_int!(i16),
        WriteFormat::I32 => put_int!(i32),
        WriteFormat::I64 => put_int!(i64),
        WriteFormat::F32 => out.extend_from_slice(&(json_f64(v)? as f32).to_le_bytes()),
        WriteFormat::F64 => out.extend_from_slice(&json_f64(v)?.to_le_bytes()),
        WriteFormat::Struct(fields) => {
            let arr = v
                .as_array()
                .ok_or_else(|| "struct format requires an array value".to_string())?;
            if arr.len() != fields.len() {
                return Err(format!(
                    "struct expects {} value(s), got {}",
                    fields.len(),
                    arr.len()
                ));
            }
            for (f, item) in fields.iter().zip(arr) {
                encode_write_value(f, item, out)?;
            }
        }
    }
    Ok(())
}

/// Build the byte payload for a write: encode `value_json` per `fmt` when a
/// format is given, otherwise treat the string as hex (the legacy behaviour).
fn write_payload(fmt: Option<&str>, value: &str) -> Result<Vec<u8>, String> {
    match fmt {
        Some(fmt_str) => {
            let wf = parse_write_format(fmt_str)?;
            let jv: serde_json::Value =
                serde_json::from_str(value).map_err(|e| format!("bad value JSON: {e}"))?;
            let mut buf = Vec::new();
            encode_write_value(&wf, &jv, &mut buf)?;
            Ok(buf)
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
            None => o.get::<_, Option<String>>("device")?.map(|s| vec![s]).unwrap_or_default(),
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
                    let ScanState::Idle { rssi, names, devices, filter_seen } =
                        std::mem::replace(&mut *guard, ScanState::Closed)
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
                        Ok(Some((peripheral, info))) => {
                            JsOutcome::ok(ScannedDevice { peripheral, info })
                        }
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
                            map.get(&n.uuid).and_then(|f| f.decode_value(&n.value).ok())
                        };
                        let value = match decoded {
                            Some(v) => CharValue::Decoded(v),
                            None => CharValue::Raw(n.value),
                        };
                        JsOutcome::ok(Notification { uuid: n.uuid, value, as_array_buffer })
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
fn device_into_js<'js>(
    ctx: &Ctx<'js>,
    peripheral: Peripheral,
    info: DeviceInfo,
) -> JsResult<Object<'js>> {
    let id = info.id.clone();
    let inner = Arc::new(Mutex::new(info));
    // Decode formats registered by subscribe(), used when notifications arrive.
    let notify_decode: NotifyDecode = Arc::new(StdMutex::new(HashMap::new()));
    // Cancel handles for every live notifications iterator, so unsubscribe()
    // can close them.
    let notif_cancels: CancelRegistry = Arc::new(StdMutex::new(Vec::new()));

    let snapshot = {
        let inner = inner.clone();
        Function::new(ctx.clone(), Async(move || {
            let inner = inner.clone();
            async move {
                match serde_json::to_string(&*inner.lock().await) {
                    Ok(j) => JsOutcome::ok(j),
                    Err(e) => JsOutcome::err(e.to_string()),
                }
            }
        }))?
    };

    let connect = {
        let (p, inner) = (peripheral.clone(), inner.clone());
        Function::new(ctx.clone(), Async(move || {
            let (p, inner) = (p.clone(), inner.clone());
            async move {
                match inner.lock().await.connect(&p).await {
                    Ok(()) => JsOutcome::<String>::empty(),
                    Err(e) => JsOutcome::err(e.to_string()),
                }
            }
        }))?
    };

    let disconnect = {
        let (p, inner) = (peripheral.clone(), inner.clone());
        Function::new(ctx.clone(), Async(move || {
            let (p, inner) = (p.clone(), inner.clone());
            async move {
                match inner.lock().await.disconnect(&p).await {
                    Ok(()) => JsOutcome::<String>::empty(),
                    Err(e) => JsOutcome::err(e.to_string()),
                }
            }
        }))?
    };

    let enumerate = {
        let (p, inner) = (peripheral.clone(), inner.clone());
        Function::new(ctx.clone(), Async(
            move |services: Opt<Vec<String>>, chars: Opt<Vec<String>>| {
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
            },
        ))?
    };

    // read(chars, as_array_buffer?) -> { uuid: value }
    // chars: ["uuid"] or ["uuid::fmt"]; fmt decodes, otherwise hex / ArrayBuffer.
    let read = {
        let (p, inner) = (peripheral.clone(), inner.clone());
        Function::new(ctx.clone(), Async(
            move |chars: Opt<Vec<String>>, as_array_buffer: Opt<bool>| {
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
                            return JsOutcome::err(format!(
                                "characteristic {uuid} not found (enumerate first?)"
                            ));
                        };
                        let bytes = match p.read(ch).await {
                            Ok(b) => b,
                            Err(e) => return JsOutcome::err(format!("read {uuid} failed: {e}")),
                        };
                        let val = match fmt {
                            Some(f) => match f.decode_value(&bytes) {
                                Ok(v) => CharValue::Decoded(v),
                                Err(_) => CharValue::Raw(bytes), // fall back to raw
                            },
                            None => CharValue::Raw(bytes),
                        };
                        items.push((uuid, val));
                    }
                    JsOutcome::ok(ReadResults { items, as_array_buffer: as_ab })
                }
            },
        ))?
    };

    // subscribe(chars) -> [uuid...]
    // chars: ["uuid"] or ["uuid::fmt"]; a fmt registers a decoder for that
    // characteristic's notifications.
    let subscribe = {
        let (p, inner, decode) = (peripheral.clone(), inner.clone(), notify_decode.clone());
        Function::new(ctx.clone(), Async(move |chars: Opt<Vec<String>>| {
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
                        return JsOutcome::err(format!(
                            "characteristic {uuid} not found (enumerate first?)"
                        ));
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
        }))?
    };

    // unsubscribe(chars) -> [uuid...]
    // chars: ["uuid"] or ["uuid::fmt"] (fmt ignored); stops notifications for
    // each characteristic and drops any decoder registered by subscribe().
    let unsubscribe = {
        let (p, inner, decode, cancels) =
            (peripheral.clone(), inner.clone(), notify_decode.clone(), notif_cancels.clone());
        Function::new(ctx.clone(), Async(move |chars: Opt<Vec<String>>| {
            let (p, inner, decode, cancels) =
                (p.clone(), inner.clone(), decode.clone(), cancels.clone());
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
                        return JsOutcome::err(format!(
                            "characteristic {uuid} not found (enumerate first?)"
                        ));
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
        }))?
    };

    // write(spec, value, withoutResponse?)
    //   spec : "uuid" or "uuid::fmt"
    //   value: with "::fmt", a JS value (scalar) or array (for struct<...>),
    //          passed through as JSON; without a fmt, the JS shim has already
    //          normalised a hex string / ArrayBuffer / Uint8Array to hex.
    let write = {
        let (p, inner) = (peripheral.clone(), inner.clone());
        Function::new(ctx.clone(), Async(
            move |spec: String, value: String, wor: Opt<bool>| {
                let (p, inner) = (p.clone(), inner.clone());
                async move {
                    let (uuid, fmt) = match parse_uuid_spec(&spec) {
                        Ok(s) => s,
                        Err(e) => return JsOutcome::<String>::err(e),
                    };
                    let bytes = match write_payload(fmt.as_deref(), &value) {
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
                        return JsOutcome::err(format!(
                            "characteristic {uuid} not found (enumerate first?)"
                        ));
                    };
                    match p.write(ch, &bytes, write_type).await {
                        Ok(()) => JsOutcome::empty(),
                        Err(e) => JsOutcome::err(format!("write {uuid} failed: {e}")),
                    }
                }
            },
        ))?
    };

    let update_rssi = {
        let (p, inner) = (peripheral.clone(), inner.clone());
        Function::new(ctx.clone(), Async(move || {
            let (p, inner) = (p.clone(), inner.clone());
            async move {
                inner.lock().await.update_rssi(&p).await;
                JsOutcome::<String>::empty()
            }
        }))?
    };

    // notifications(as_array_buffer?) -> breakable async-iterable (returns sync)
    let notifications = {
        let (peripheral, decode, cancels) =
            (peripheral.clone(), notify_decode.clone(), notif_cancels.clone());
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
                read:       m2("read"),
                subscribe:  m("subscribe"),
                unsubscribe: m("unsubscribe"),
                // write(spec, value, withoutResponse?). With "::fmt" the value is
                // sent as JSON (scalar or array); otherwise it's normalised to hex
                // (hex string | ArrayBuffer | Uint8Array).
                write: (spec, value, withoutResponse) =>
                    raw.write(
                        spec,
                        spec.includes("::") ? JSON.stringify(value) : toHex(value),
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