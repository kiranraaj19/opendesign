//! OpenDesign's MCP server: lets an AI assistant (Claude, say) design
//! garments, drape them on an avatar and render them, without the app's
//! window. It speaks the Model Context Protocol over stdio: one JSON-RPC
//! message per line on stdin, replies one per line on stdout, logs on
//! stderr.
//!
//! The work happens on an ordinary OpenDesign document through the same
//! commands the app uses, so every change can be undone and a saved
//! project opens in the app as usual.

mod garment;
mod render;
mod session;
mod tools;

use std::io::{BufRead, Write};

use serde_json::{json, Value};

use session::Session;

/// Protocol versions this server can speak, newest first.
const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

/// What the assistant is told about working with this server, once, when
/// it connects.
const INSTRUCTIONS: &str = "\
OpenDesign is a garment design program: flat pattern pieces are drafted in \
2D, sewn together, draped on an avatar by a cloth simulation, and rendered.

Units and axes: pattern points are in millimetres, x to the right, y DOWN \
(as on paper). Outline points are numbered from 0; a seam edge runs between \
two of them. Curve points are control points the outline bends towards.

A good workflow:
1. create_garment for a ready-made, sewn and arranged starting point \
(tshirt, skirt or dress), or add_pattern pieces yourself, then sew and arrange them.
2. get_document / get_pattern to see the pieces, their point indices and \
edge lengths, and edit to change them (many operations per call).
3. drape, then render (front, back, side or three_quarter views; \
style=strain shows where the fabric is stretched) and look at the picture.
4. Adjust, drape and render again; save_project and export when done.

Sewing: each seam side runs from one point to another; the two sides are \
joined start to start, end to end. Sew a front edge running neck to \
shoulder to a back edge also running neck to shoulder.";

fn main() {
    let mut server = Server::new();
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(message) => server.handle(message),
            Err(e) => Some(error(Value::Null, -32700, &format!("parse error: {e}"))),
        };
        if let Some(reply) = reply {
            // A closed stdout means the client has gone.
            if writeln!(stdout, "{reply}")
                .and_then(|_| stdout.flush())
                .is_err()
            {
                break;
            }
        }
    }
}

/// The MCP side: routes JSON-RPC requests to [`tools`].
pub struct Server {
    session: Session,
}

impl Server {
    pub fn new() -> Self {
        Self {
            session: Session::new(),
        }
    }
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

impl Server {
    /// The reply to one message, or `None` for a notification (or a batch
    /// of nothing but notifications).
    pub fn handle(&mut self, message: Value) -> Option<Value> {
        if let Value::Array(batch) = message {
            let replies: Vec<Value> = batch.into_iter().filter_map(|m| self.handle(m)).collect();
            return (!replies.is_empty()).then_some(Value::Array(replies));
        }
        let id = message.get("id").cloned();
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        // Notifications (no id) get no reply; replies to requests we never
        // send are ignored the same way.
        let id = id?;
        Some(match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str);
                let version = asked
                    .filter(|v| PROTOCOL_VERSIONS.contains(v))
                    .unwrap_or(PROTOCOL_VERSIONS[0]);
                result(
                    id,
                    json!({
                        "protocolVersion": version,
                        "capabilities": { "tools": { "listChanged": false } },
                        "serverInfo": {
                            "name": "opendesign",
                            "title": "OpenDesign",
                            "version": env!("CARGO_PKG_VERSION"),
                        },
                        "instructions": INSTRUCTIONS,
                    }),
                )
            }
            "ping" => result(id, json!({})),
            "tools/list" => result(id, json!({ "tools": tools::list() })),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                match tools::call(&mut self.session, name, &args) {
                    Some(outcome) => result(id, outcome.into_json()),
                    None => error(id, -32602, &format!("unknown tool: {name}")),
                }
            }
            _ => error(id, -32601, &format!("method not found: {method}")),
        })
    }
}

fn result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(server: &mut Server, id: u64, method: &str, params: Value) -> Value {
        server
            .handle(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .expect("a request gets a reply")
    }

    #[test]
    fn handshake_lists_tools_and_ignores_notifications() {
        let mut server = Server::new();
        let init = call(
            &mut server,
            1,
            "initialize",
            json!({ "protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": { "name": "test", "version": "0" } }),
        );
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert!(init["result"]["capabilities"]["tools"].is_object());
        // An unknown version gets the newest we speak.
        let init = call(
            &mut server,
            2,
            "initialize",
            json!({ "protocolVersion": "1999-01-01" }),
        );
        assert_eq!(init["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
        assert!(server
            .handle(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .is_none());

        let list = call(&mut server, 3, "tools/list", json!({}));
        let tools = list["result"]["tools"].as_array().unwrap();
        assert!(tools.len() >= 10);
        for tool in tools {
            assert!(tool["name"].is_string() && tool["description"].is_string());
            assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
        }
        let missing = call(&mut server, 4, "nope", json!({}));
        assert_eq!(missing["error"]["code"], -32601);
        let unknown = call(&mut server, 5, "tools/call", json!({ "name": "nope" }));
        assert_eq!(unknown["error"]["code"], -32602);
    }

    #[test]
    fn a_tool_error_is_reported_as_a_result_not_a_protocol_error() {
        let mut server = Server::new();
        let reply = call(
            &mut server,
            1,
            "tools/call",
            json!({ "name": "get_pattern", "arguments": { "pattern": 99 } }),
        );
        assert_eq!(reply["result"]["isError"], true);
        let text = reply["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("99"), "{text}");
    }
}
