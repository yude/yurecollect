use std::collections::VecDeque;
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse},
    routing::{get},
    Router,
};
use axum::extract::ws::{Message as WsMessage, WebSocketUpgrade};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::{broadcast, RwLock};
use tokio::time::{sleep, Duration};
use tokio_tungstenite::connect_async;

const MAX_BUFFER_BYTES: usize = 1024 * 1024 * 1024 * 1; // 1GB

struct MessageBuffer {
    total_bytes: usize,
    entries: VecDeque<String>,
}

impl MessageBuffer {
    fn new() -> Self {
        Self {
            total_bytes: 0,
            entries: VecDeque::new(),
        }
    }

    fn push(&mut self, msg: String) {
        let msg_len = msg.len();
        while self.total_bytes + msg_len > MAX_BUFFER_BYTES {
            if let Some(front) = self.entries.pop_front() {
                self.total_bytes = self.total_bytes.saturating_sub(front.len());
            } else {
                break;
            }
        }
        self.total_bytes += msg_len;
        self.entries.push_back(msg);
    }

    fn len(&self) -> usize { self.entries.len() }
    fn iter(&self) -> impl DoubleEndedIterator<Item=&String> { self.entries.iter() }
}

#[derive(Clone)]
struct AppState {
    buffer: Arc<RwLock<MessageBuffer>>,
    tx: broadcast::Sender<String>,
}

#[derive(Deserialize)]
struct ListParams { limit: Option<usize> }

#[tokio::main]
async fn main() {
    // Get WebSocket URL from CLI arg or env var
    let url = env::args().nth(1).or_else(|| env::var("WS_URL").ok());
    let Some(url) = url else {
        eprintln!("Usage: yurecollect <ws-url>\nAlternatively set WS_URL env var.");
        std::process::exit(2);
    };

    let state = AppState {
        buffer: Arc::new(RwLock::new(MessageBuffer::new())),
        tx: broadcast::channel(1024).0,
    };

    // Spawn HTTP server for web UI
    let state_for_http = state.clone();
    let mut http_task = tokio::spawn(async move {
        run_http_server(state_for_http).await;
    });

    // Connect to upstream websocket and stream messages
    let state_for_ws = state.clone();
    let mut ws_task = tokio::spawn(async move {
        run_upstream_ws(url, state_for_ws).await;
    });

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            eprintln!("Received Ctrl+C, shutting down...");
            http_task.abort();
            ws_task.abort();
        }
        _ = &mut http_task => {
            eprintln!("HTTP task ended, shutting down...");
            ws_task.abort();
        }
        _ = &mut ws_task => {
            eprintln!("Upstream task ended, shutting down...");
            http_task.abort();
        }
    }
}

async fn run_upstream_ws(url: String, state: AppState) {
    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(30);

    loop {
        let (ws_stream, _resp) = match connect_async(&url).await {
            Ok(pair) => {
                eprintln!("Connected to upstream: {}", url);
                backoff = Duration::from_secs(1);
                pair
            }
            Err(err) => {
                eprintln!(
                    "Failed to connect to {}: {} (retry in {:?})",
                    url, err, backoff
                );
                sleep(backoff).await;
                backoff = std::cmp::min(backoff * 2, max_backoff);
                continue;
            }
        };

        let (_write, mut read) = ws_stream.split();

        while let Some(item) = read.next().await {
            match item {
                Ok(msg) => {
                    if msg.is_text() {
                        let text = msg.into_text().unwrap_or_default();

                        // Print raw message to stdout
                        println!("{}", text);

                        // Store message in in-memory buffer capped at ~1GB
                        {
                            let mut buf = state.buffer.write().await;
                            buf.push(text.clone());
                        }

                        // Publish to subscribers
                        let _ = state.tx.send(text.clone());

                        // Try to parse JSON to validate
                        let _ = serde_json::from_str::<Value>(&text).map_err(|e| {
                            eprintln!("JSON parse error: {}", e);
                        });
                    } else if msg.is_binary() {
                        let bin = msg.into_data();
                        println!("<binary message: {} bytes>", bin.len());
                        {
                            let mut buf = state.buffer.write().await;
                            buf.push(format!("<binary {} bytes>", bin.len()));
                        }
                        let _ = state.tx.send(format!("<binary {} bytes>", bin.len()));
                    } else if msg.is_close() {
                        eprintln!("Upstream WebSocket closed. reconnecting...");
                        break;
                    } else {
                        // ignore
                    }
                }
                Err(err) => {
                    eprintln!(
                        "WebSocket read error: {} (reconnect in {:?})",
                        err, backoff
                    );
                    break;
                }
            }
        }

        sleep(backoff).await;
        backoff = std::cmp::min(backoff * 2, max_backoff);
    }
}

async fn run_http_server(state: AppState) {
    let app = Router::new()
        .route("/", get(index))
        .route("/api/messages", get(list_messages))
        .route("/ws", get(ws_handler))
        .with_state(state);

    let addr: SocketAddr = ([0, 0, 0, 0], 3000).into();
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    println!("Web UI available at http://{}/", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}

async fn index() -> impl IntoResponse {
    Html(INDEX_HTML)
}

async fn list_messages(State(state): State<AppState>, Query(p): Query<ListParams>) -> impl IntoResponse {
    let limit = p.limit.unwrap_or(500);
    let buf = state.buffer.read().await;
    let total = buf.len();
    let start = total.saturating_sub(limit);
    let slice: Vec<String> = buf.iter().skip(start).cloned().collect();
    axum::Json(slice)
}

async fn ws_handler(State(state): State<AppState>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |mut socket| async move {
        let mut rx = state.tx.subscribe();
        while let Ok(msg) = rx.recv().await {
            if socket.send(WsMessage::Text(msg)).await.is_err() {
                break;
            }
        }
    })
}

// Simple embedded HTML for the frontend
const INDEX_HTML: &str = r#"<!doctype html>
<html lang="ja">
<head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>yurecollect</title>
    <link rel="stylesheet" href="https://unpkg.com/uplot@1.6.27/dist/uPlot.min.css" />
    <style>
        :root { color-scheme: light dark; }
        body { font-family: system-ui, sans-serif; margin: 0; }
        header { padding: 12px 16px; border-bottom: 1px solid #8884; display:flex; gap:12px; align-items:center; }
        main { padding: 12px 16px; display: grid; gap: 16px; }
        #chart { width: 100%; height: 320px; }
        .item { padding: 6px 8px; border: 1px solid #8884; border-radius: 6px; white-space: pre-wrap; font-family: ui-monospace, Menlo, monospace; font-size: 12px; }
        .meta { color: #888; font-size: 12px; }
    </style>
    <script src="https://unpkg.com/uplot@1.6.27/dist/uPlot.iife.min.js"></script>
    <script>
        async function boot() {
            // let logEl = document.getElementById('log');
            let chartEl = document.getElementById('chart');
            if (!chartEl) {
                // Fallback: create chart container if missing
                chartEl = document.createElement('div');
                chartEl.id = 'chart';
                chartEl.style.width = '100%';
                chartEl.style.height = '320px';
                const mainEl = document.querySelector('main') || document.body;
                mainEl.prepend(chartEl);
            }
            // if (!logEl) {
            //     const mainEl = document.querySelector('main') || document.body;
            //     logEl = document.createElement('div');
            //     logEl.id = 'log';
            //     mainEl.append(logEl);
            // }
            // logs: newest at top, keep latest ~10 lines

            // uPlot data buffers (per UA series)
            const tArr = [];  // timestamps (seconds)
            const uaSeries = new Map(); // ua -> { ax:[], ay:[], az:[] }
            const uaOrder = [];
            const MAX_POINTS = 20000;
            const WINDOW_SECONDS = 3600; // show last 60s; right edge anchored to now
            const UPDATE_INTERVAL_MS = 100; // throttle graph updates
            let updateScheduled = false;

            function scheduleUpdate() {
                if (!updateScheduled) {
                    updateScheduled = true;
                    setTimeout(() => {
                        updateScheduled = false;
                        if (u) u.setData(dataMatrix());
                    }, UPDATE_INTERVAL_MS);
                }
            }

            const UA_COLORS = ['#e11d48','#22c55e','#3b82f6','#f59e0b','#a78bfa','#14b8a6','#ef4444','#10b981'];
            const uaColorMap = new Map();
            function getUAColor(ua) {
                if (!uaColorMap.has(ua)) {
                    const idx = uaColorMap.size % UA_COLORS.length;
                    uaColorMap.set(ua, UA_COLORS[idx]);
                }
                return uaColorMap.get(ua);
            }
            function ensureUA(ua) {
                if (!uaSeries.has(ua)) {
                    uaOrder.push(ua);
                    uaSeries.set(ua, { ax: [], ay: [], az: [] });
                    const len = tArr.length;
                    const s = uaSeries.get(ua);
                    for (let i = 0; i < len; i++) { s.ax.push(null); s.ay.push(null); s.az.push(null); }
                    rebuildPlot();
                }
            }

            let u = null;
            function seriesForUA(ua) {
                const color = getUAColor(ua);
                return [
                    { label: `${ua} ax`, stroke: color, points: { show: true, size: 3 } },
                    { label: `${ua} ay`, stroke: color, points: { show: true, size: 3 } },
                    { label: `${ua} az`, stroke: color, points: { show: true, size: 3 } },
                ];
            }
            function dataMatrix() {
                const data = [tArr];
                for (const ua of uaOrder) {
                    const s = uaSeries.get(ua);
                    data.push(s.ax, s.ay, s.az);
                }
                return data;
            }
            function rebuildPlot() {
                const opts = {
                    title: 'yure',
                    width: chartEl.clientWidth || window.innerWidth,
                    height: chartEl.clientHeight || 320,
                    scales: {
                        x: {
                            time: true,
                            // Left edge: auto (data min), Right edge: browser now
                            range: (u, min, _max) => {
                                const now = Date.now() / 1000;
                                return [min, now];
                            },
                        },
                        // y: { range: [-2.0, 2.0] },
                    },
                    axes: [
                        { grid: { show: true } },
                        { grid: { show: true }, label: 'acc' },
                    ],
                    series: [ {} ].concat(uaOrder.flatMap(seriesForUA)),
                };
                if (u) u.destroy();
                u = new uPlot(opts, dataMatrix(), chartEl);
            }

            // Resize handling
            addEventListener('resize', () => {
                if (u) u.setSize({ width: chartEl.clientWidth, height: chartEl.clientHeight || 320 });
            });

            function toNum(v) { const n = Number(v); return Number.isFinite(n) ? n : null; }
            function toTsSeconds(t) {
                const ms = Number(t);
                return Number.isFinite(ms) ? (ms / 1000) : (Date.now() / 1000);
            }

            function pushData(t, x, y, z, ua) {
                const nowSec = Date.now() / 1000;
                const ts = toTsSeconds(t);
                if (ts > nowSec) return; // ignore future samples
                const uaKey = String(ua ?? 'unknown');
                ensureUA(uaKey);
                tArr.push(ts);
                // append nulls for all UA, then set target UA values
                for (const key of uaOrder) {
                    const s = uaSeries.get(key);
                    s.ax.push(null); s.ay.push(null); s.az.push(null);
                }
                const idx = tArr.length - 1;
                const sTarget = uaSeries.get(uaKey);
                sTarget.ax[idx] = toNum(x);
                sTarget.ay[idx] = toNum(y);
                sTarget.az[idx] = toNum(z);
                // do not trim by time window; keep all until MAX_POINTS
                while (tArr.length > MAX_POINTS) {
                    tArr.shift();
                    for (const key of uaOrder) {
                        const s = uaSeries.get(key);
                        s.ax.shift(); s.ay.shift(); s.az.shift();
                    }
                }
                scheduleUpdate();
            }

            // Initial fetch of recent messages
            try {
                const res = await fetch('/api/messages?limit=500');
                const arr = await res.json();
                arr.forEach(addItem);
            } catch (e) { console.error(e); }

            // Live updates via WebSocket
            const proto = location.protocol === 'https:' ? 'wss' : 'ws';
            const wsUrl = proto + '://' + location.host + '/ws';
            let ws = null;
            let reconnectTimer = null;
            let reconnectDelayMs = 500;
            const reconnectDelayMaxMs = 30_000;
            let manuallyClosed = false;

            function scheduleReconnect() {
                if (manuallyClosed) return;
                if (reconnectTimer != null) return;
                const delay = reconnectDelayMs;
                reconnectDelayMs = Math.min(reconnectDelayMs * 2, reconnectDelayMaxMs);
                console.warn(`ws disconnected; retry in ${delay}ms`);
                reconnectTimer = setTimeout(() => {
                    reconnectTimer = null;
                    connectWs();
                }, delay);
            }

            function connectWs() {
                if (manuallyClosed) return;
                try {
                    ws = new WebSocket(wsUrl);
                } catch (e) {
                    console.error(e);
                    scheduleReconnect();
                    return;
                }
                ws.onopen = () => {
                    reconnectDelayMs = 500;
                    console.info('ws connected');
                };
                ws.onmessage = (ev) => {
                    try { addItem(ev.data); } catch (e) { console.error(e); }
                };
                ws.onerror = () => {
                    // Most browsers also emit onclose; close() forces a clean state.
                    try { ws.close(); } catch {}
                };
                ws.onclose = () => {
                    scheduleReconnect();
                };
            }

            addEventListener('beforeunload', () => {
                manuallyClosed = true;
                if (reconnectTimer != null) {
                    clearTimeout(reconnectTimer);
                    reconnectTimer = null;
                }
                try { ws?.close(); } catch {}
            });

            connectWs();

            function addItem(text) {
                // If message is JSON array, expand into multiple tiles and update chart
                try {
                    const parsed = JSON.parse(text);
                    if (Array.isArray(parsed)) {
                        for (const item of parsed) {
                            const t = item.t ?? item.time ?? Date.now();
                            const x = item.x ?? item.ax ?? item.accelerationX ?? item.acceleration?.x ?? null;
                            const y = item.y ?? item.ay ?? item.accelerationY ?? item.acceleration?.y ?? null;
                            const z = item.z ?? item.az ?? item.accelerationZ ?? item.acceleration?.z ?? null;
                            pushData(t, x, y, z, item.userAgent);
                            // prependLog(JSON.stringify(item));
                        }
                        return;
                    } else if (parsed && typeof parsed === 'object') {
                        const t = parsed.t ?? parsed.time ?? Date.now();
                        const x = parsed.x ?? parsed.ax ?? parsed.accelerationX ?? parsed.acceleration?.x ?? null;
                        const y = parsed.y ?? parsed.ay ?? parsed.accelerationY ?? parsed.acceleration?.y ?? null;
                        const z = parsed.z ?? parsed.az ?? parsed.accelerationZ ?? parsed.acceleration?.z ?? null;
                        pushData(t, x, y, z, parsed.userAgent);
                        // prependLog(JSON.stringify(parsed));
                        return;
                    }
                } catch {}
                // prependLog(text);
            }

            function prependLog(content) {
                const el = document.createElement('div');
                el.className = 'item';
                el.textContent = content;
                // newest at top
                logEl.insertBefore(el, logEl.firstChild);
                // keep only latest ~10 lines
                while (logEl.childNodes.length > 10) {
                    logEl.removeChild(logEl.lastChild);
                }
            }
        }
        addEventListener('DOMContentLoaded', boot);
    </script>
    </head>
    <body>
        <header>
            <h1 style="margin: 0">yurecollect</h1>
        </header>
        <main>
            <div id="chart"></div>
            <!-- <div id="log" aria-label="recent logs" style="margin-top: 20em;"></div> -->
        </main>
    </body>
    </html>"#;
