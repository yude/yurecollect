use std::collections::VecDeque;
use std::env;

use futures_util::StreamExt;
use serde_json::Value;
use tokio_tungstenite::connect_async;

const MAX_BUFFER_BYTES: usize = 1024 * 1024 * 1024; // 1GB

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
        // Evict oldest messages until we have space for the new one
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
}

#[tokio::main]
async fn main() {
    // Get WebSocket URL from CLI arg or env var
    let url = env::args().nth(1).or_else(|| env::var("WS_URL").ok());

    let Some(url) = url else {
        eprintln!("Usage: yurecollect <ws-url>\nAlternatively set WS_URL env var.");
        std::process::exit(2);
    };

    let (ws_stream, _resp) = match connect_async(&url).await {
        Ok(pair) => pair,
        Err(err) => {
            eprintln!("Failed to connect to {}: {}", url, err);
            std::process::exit(1);
        }
    };

    let (_write, mut read) = ws_stream.split();

    let mut buffer = MessageBuffer::new();

    while let Some(item) = read.next().await {
        match item {
            Ok(msg) => {
                if msg.is_text() {
                    let text = msg.into_text().unwrap_or_default();

                    // Print raw message to stdout
                    println!("{}", text);

                    // Store message in in-memory buffer capped at ~1GB
                    buffer.push(text.clone());

                    // Try to parse JSON (array or object) just to "read" it
                    // Parsing errors won't stop the program
                    let _ = serde_json::from_str::<Value>(&text).map_err(|e| {
                        eprintln!("JSON parse error: {}", e);
                    });
                } else if msg.is_binary() {
                    let bin = msg.into_data();
                    println!("<binary message: {} bytes>", bin.len());
                    buffer.push(format!("<binary {} bytes>", bin.len()));
                } else {
                    // Other message types (ping/pong/close) are ignored here
                }
            }
            Err(err) => {
                eprintln!("WebSocket read error: {}", err);
                break;
            }
        }
    }
}
