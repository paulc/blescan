use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{Context, anyhow};
use axum::Router;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::post;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use crate::commands::{Commands, SseArgs};

// Commands available via SSE (js/run/sse never allowed)
const COMMANDS: [&str; 8] = [
    "scan",
    "monitor",
    "enumerate",
    "poll",
    "notify",
    "write",
    "write_read",
    "dump",
];

const INDEX_HTML: &str = include_str!("sse/index.html");
const VIEW_HTML: &str = include_str!("sse/view.html");

struct AppState {
    allow: HashSet<String>,
    global_args: Vec<String>,
    pages: HashMap<String, String>,
    cors: bool,
    debug: bool,
}

pub async fn run(args: SseArgs, global_args: Vec<String>) -> anyhow::Result<()> {
    let allow = if args.allow.is_empty() {
        COMMANDS.iter().map(|s| s.to_string()).collect::<HashSet<_>>()
    } else {
        let mut allow = HashSet::new();
        for c in args.allow.iter().flat_map(|s| s.split(',')) {
            let c = c.trim().replace('-', "_");
            if !COMMANDS.contains(&c.as_str()) {
                return Err(anyhow!("Invalid command: {c}"));
            }
            allow.insert(c);
        }
        allow
    };

    let endpoint = if args.endpoint.starts_with('/') {
        args.endpoint.clone()
    } else {
        format!("/{}", args.endpoint)
    };

    // Built-in pages (keep COMMANDS order)
    let commands = COMMANDS.iter().filter(|c| allow.contains(**c)).collect::<Vec<_>>();
    let subst = |s: &str| {
        s.replace("{{ENDPOINT}}", &json!(endpoint).to_string())
            .replace("{{COMMANDS}}", &json!(commands).to_string())
    };
    let mut pages = HashMap::new();
    pages.insert("/debug/index.html".to_string(), subst(INDEX_HTML));
    pages.insert("/debug/view.html".to_string(), subst(VIEW_HTML));
    let index = match &args.index {
        Some(f) => std::fs::read_to_string(f).with_context(|| format!("Error reading index: {f}"))?,
        None => subst(INDEX_HTML),
    };
    pages.insert("/".to_string(), index.clone());
    pages.insert("/index.html".to_string(), index);
    for h in &args.html {
        let (path, file) = h.split_once(':').ok_or(anyhow!("Invalid html arg (path:file): {h}"))?;
        let html = std::fs::read_to_string(file).with_context(|| format!("Error reading html: {file}"))?;
        pages.insert(path.to_string(), html);
    }

    let state = Arc::new(AppState {
        allow,
        global_args,
        pages,
        cors: args.cors,
        debug: args.debug,
    });

    let app = Router::new()
        .route(&endpoint, post(sse_handler).options(preflight_handler))
        .fallback(page_handler)
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind(&args.bind).await?;
    eprintln!("Listening on http://{}", args.bind);
    eprintln!("SSE endpoint: http://{}{} (POST)", args.bind, endpoint);
    eprintln!(
        "Commands: {}",
        commands.iter().map(|s| **s).collect::<Vec<_>>().join(",")
    );
    if args.index.is_some() {
        eprintln!("Debug page: http://{}/debug/index.html", args.bind);
    } else {
        eprintln!("Debug page: http://{}/", args.bind);
    }

    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;
    Ok(())
}

async fn page_handler(State(state): State<Arc<AppState>>, uri: Uri) -> Response {
    match state.pages.get(uri.path()) {
        Some(html) => Html(html.clone()).into_response(),
        None => (StatusCode::NOT_FOUND, "Not Found\n").into_response(),
    }
}

fn add_cors(headers: &mut HeaderMap) {
    headers.insert("access-control-allow-origin", HeaderValue::from_static("*"));
    headers.insert(
        "access-control-allow-methods",
        HeaderValue::from_static("POST, OPTIONS"),
    );
    headers.insert("access-control-allow-headers", HeaderValue::from_static("*"));
}

async fn preflight_handler(State(state): State<Arc<AppState>>) -> Response {
    if !state.cors {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    add_cors(response.headers_mut());
    response
}

// Force JSON output and return command name (None if not available via SSE)
fn set_json(cmd: &mut Commands) -> Option<&'static str> {
    let (json, name) = match cmd {
        Commands::Scan(a) => (&mut a.json, "scan"),
        Commands::Monitor(a) => (&mut a.json, "monitor"),
        Commands::Enumerate(a) => (&mut a.json, "enumerate"),
        Commands::Poll(a) => (&mut a.json, "poll"),
        Commands::Notify(a) => (&mut a.json, "notify"),
        Commands::Write(a) => (&mut a.json, "write"),
        Commands::WriteRead(a) => (&mut a.json, "write_read"),
        Commands::Dump(a) => (&mut a.json, "dump"),
        _ => return None,
    };
    *json = true;
    Some(name)
}

async fn sse_handler(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Response {
    let mut response = match spawn_command(&state, &body).await {
        Ok((child, name)) => {
            if state.debug {
                eprintln!("[sse] {addr}: start {name} {}", String::from_utf8_lossy(&body));
            }
            let (tx, rx) = mpsc::channel(64);
            tokio::spawn(stream_child(child, tx, addr, state.debug));
            let stream = futures::stream::unfold(rx, |mut rx| async {
                rx.recv().await.map(|e| (Ok::<Event, Infallible>(e), rx))
            });
            Sse::new(stream).keep_alive(KeepAlive::default()).into_response()
        }
        Err((status, msg)) => {
            if state.debug {
                eprintln!("[sse] {addr}: error {msg}");
            }
            (status, format!("{msg}\n")).into_response()
        }
    };
    if state.cors {
        add_cors(response.headers_mut());
    }
    response
}

// Validate command and fork `blescan run -` with command JSON on stdin
async fn spawn_command(state: &AppState, body: &[u8]) -> Result<(Child, &'static str), (StatusCode, String)> {
    let mut cmd: Commands =
        serde_json::from_slice(body).map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid command: {e}")))?;
    let name = set_json(&mut cmd)
        .filter(|n| state.allow.contains(*n))
        .ok_or((StatusCode::FORBIDDEN, "Command not allowed".to_string()))?;
    let err = |e: std::io::Error| (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {e}"));
    let mut child = Command::new(std::env::current_exe().map_err(err)?)
        .args(&state.global_args)
        .args(["run", "--", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(err)?;
    let mut stdin = child.stdin.take().expect("stdin piped");
    let json = serde_json::to_vec(&cmd).expect("serialise command");
    stdin.write_all(&json).await.map_err(err)?;
    Ok((child, name))
}

// Forward child stdout (NDJSON) as `message` events and stderr as `stderr`
// events (JSON string), then send `exit` event. Child killed on client
// disconnect (kill_on_drop).
async fn stream_child(mut child: Child, tx: mpsc::Sender<Event>, addr: SocketAddr, debug: bool) {
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout piped")).lines();
    let mut stderr = BufReader::new(child.stderr.take().expect("stderr piped")).lines();
    let stderr_event = |l: String| Event::default().event("stderr").data(json!(l).to_string());
    let mut stderr_open = true;
    loop {
        let event = tokio::select! {
            _ = tx.closed() => {
                if debug {
                    eprintln!("[sse] {addr}: client disconnected");
                }
                return;
            }
            line = stdout.next_line() => match line {
                Ok(Some(l)) => Event::default().data(l),
                _ => break,
            },
            line = stderr.next_line(), if stderr_open => match line {
                Ok(Some(l)) => stderr_event(l),
                _ => {
                    stderr_open = false;
                    continue;
                }
            },
        };
        if tx.send(event).await.is_err() {
            return;
        }
    }
    // Drain stderr (exit errors)
    while stderr_open && let Ok(Some(l)) = stderr.next_line().await {
        if tx.send(stderr_event(l)).await.is_err() {
            return;
        }
    }
    let code = child.wait().await.ok().and_then(|s| s.code());
    if debug {
        eprintln!("[sse] {addr}: exit {code:?}");
    }
    let _ = tx
        .send(Event::default().event("exit").data(json!({ "code": code }).to_string()))
        .await;
}
