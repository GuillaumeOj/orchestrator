//! `orch-mcp`: the orchestrator's MCP server, run by Claude Code over stdio
//! for each agent (SPEC §6, ADR-009 decision 5). It speaks JSON-RPC 2.0, one
//! message per line, and forwards each tool call to the core over the bus;
//! the agent's identity comes from the environment the core set at spawn.
//!
//! P1 tools:
//! - `commit(message, paths?)`: the core commits in the agent's worktree,
//!   outside the sandbox (sandboxed git can't write the repository's git dir
//!   under Claude Code 2.1.302), logged and subject to permission rules;
//! - `request_pr(title, body?, draft?)`: asks the user to push the branch and
//!   open a PR (an Inbox gate). Agents never push themselves.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::time::Duration;

use orch_protocol::{CONNECT_TIMEOUT_MS, DEFAULT_MCP_DEADLINE_MS, Envelope, McpMsg, Msg, PROTOCOL_VERSION, Reply, ReplyEnvelope, bus};
use serde_json::{Value, json};

/// Protocol revisions this server accepts; it answers with the client's when listed.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

#[derive(Clone, Debug, Default)]
pub struct McpEnv {
    pub agent_id: Option<String>,
    pub socket: Option<PathBuf>,
    pub deadline: Duration,
}

impl McpEnv {
    pub fn from_env() -> McpEnv {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        McpEnv {
            agent_id: var(orch_protocol::env::AGENT_ID),
            socket: var(orch_protocol::env::SOCKET).map(PathBuf::from),
            deadline: Duration::from_millis(DEFAULT_MCP_DEADLINE_MS),
        }
    }
}

/// The tool list (names are what Claude Code shows as `mcp__orch__<name>`).
pub fn tools() -> Value {
    json!([
        {
            "name": "commit",
            "description": "Commit changes in your worktree. Use this instead of `git commit`: the sandbox blocks git from writing the repository. Stages the given paths (or every change when `paths` is omitted, like `git add -A`) and commits them on your branch. Hooks do not run. It never pushes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "message": {"type": "string", "description": "The commit message (first line = summary)."},
                    "paths": {"type": "array", "items": {"type": "string"}, "description": "Paths relative to the worktree root. Omit to commit everything."}
                },
                "required": ["message"],
                "additionalProperties": false
            }
        },
        {
            "name": "request_pr",
            "description": "Ask the user to push your branch and open a pull request. You cannot push yourself. This creates an approval item for the user and returns at once; you are told the result later.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": {"type": "string"},
                    "body": {"type": "string"},
                    "draft": {"type": "boolean"}
                },
                "required": ["title"],
                "additionalProperties": false
            }
        }
    ])
}

fn result(id: &Value, r: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": r})
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn tool_text(text: &str, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": is_error})
}

/// Answers one JSON-RPC message; `None` for notifications.
pub fn handle(msg: &Value, env: &McpEnv, call: &dyn Fn(&str, &Value) -> (bool, String)) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    Some(match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
            let v = if PROTOCOL_VERSIONS.contains(&asked) { asked } else { PROTOCOL_VERSIONS[0] };
            result(
                &id,
                json!({
                    "protocolVersion": v,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "orch", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": "Orchestrator tools. Commit with `commit` (git commit is blocked by the sandbox). Ask for a PR with `request_pr`."
                }),
            )
        }
        "ping" => result(&id, json!({})),
        "tools/list" => result(&id, json!({"tools": tools()})),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            if !tools().as_array().is_some_and(|t| t.iter().any(|x| x["name"] == name)) {
                return Some(error(&id, -32602, &format!("unknown tool {name:?}")));
            }
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            let (is_error, text) = call(name, &args);
            let _ = env;
            result(&id, tool_text(&text, is_error))
        }
        "resources/list" => result(&id, json!({"resources": []})),
        "prompts/list" => result(&id, json!({"prompts": []})),
        _ => error(&id, -32601, &format!("method not found: {method}")),
    })
}

/// Sends one tool call to the core. Any failure is a tool error for the
/// model (never a crash of the server).
pub fn call_core(env: &McpEnv, tool: &str, args: &Value) -> (bool, String) {
    let Some(socket) = &env.socket else { return (true, "orchestrator unavailable (ORCH_SOCK not set)".into()) };
    let Some(agent_id) = &env.agent_id else { return (true, "orchestrator unavailable (ORCH_AGENT_ID not set)".into()) };
    let r = (|| -> Result<(bool, String), String> {
        let s = bus::connect(socket, Duration::from_millis(CONNECT_TIMEOUT_MS)).map_err(|e| e.tag())?;
        let m = Envelope::new(Msg::Mcp(McpMsg {
            agent_id: agent_id.clone(),
            tool: tool.into(),
            arguments: args.clone(),
            deadline_ms: env.deadline.as_millis() as u64,
        }));
        bus::write_line(&mut &s, &m).map_err(|e| e.tag())?;
        s.set_read_timeout(Some(env.deadline)).map_err(|e| e.to_string())?;
        let reply: ReplyEnvelope = bus::read_line(&mut BufReader::new(&s)).map_err(|e| e.tag())?;
        if reply.v != PROTOCOL_VERSION {
            return Err(format!("protocol version {}", reply.v));
        }
        match reply.reply {
            Reply::Mcp { is_error, text, .. } => Ok((is_error, text)),
            Reply::Error { message } => Err(message),
            other => Err(format!("unexpected reply {other:?}")),
        }
    })();
    r.unwrap_or_else(|e| (true, format!("orchestrator unavailable: {e}. Nothing was committed or requested.")))
}

/// The stdio loop: one JSON-RPC message per line in, one per line out.
pub fn serve(input: impl BufRead, mut output: impl Write, env: &McpEnv) {
    let call = |tool: &str, args: &Value| call_core(env, tool, args);
    for line in input.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(Value::Array(batch)) => {
                let rs: Vec<Value> = batch.iter().filter_map(|m| handle(m, env, &call)).collect();
                if rs.is_empty() { None } else { Some(Value::Array(rs)) }
            }
            Ok(m) => handle(&m, env, &call),
            Err(e) => Some(error(&Value::Null, -32700, &format!("parse error: {e}"))),
        };
        if let Some(r) = reply {
            if writeln!(output, "{r}").and_then(|_| output.flush()).is_err() {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn run(lines: &[Value], env: &McpEnv) -> Vec<Value> {
        let input = lines.iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n");
        let mut out = vec![];
        serve(Cursor::new(input), &mut out, env);
        String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    #[test]
    fn handshake_list_and_unknowns() {
        let env = McpEnv::default();
        let out = run(
            &[
                json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"claude-code","version":"2.1.302"}}}),
                json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
                json!({"jsonrpc":"2.0","id":3,"method":"nope"}),
                json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"rm_rf","arguments":{}}}),
                json!({"jsonrpc":"2.0","id":5,"method":"ping"}),
            ],
            &env,
        );
        assert_eq!(out.len(), 5, "no reply to the notification");
        assert_eq!(out[0]["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(out[0]["result"]["serverInfo"]["name"], "orch");
        let names: Vec<_> = out[1]["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].clone()).collect();
        assert_eq!(names, ["commit", "request_pr"]);
        assert_eq!(out[2]["error"]["code"], -32601);
        assert_eq!(out[3]["error"]["code"], -32602);
        assert_eq!(out[4]["result"], json!({}));
    }

    #[test]
    fn core_down_is_a_tool_error_not_a_crash() {
        let d = tempfile::Builder::new().prefix("om").tempdir_in("/tmp").unwrap();
        let env = McpEnv {
            agent_id: Some("a1".into()),
            socket: Some(d.path().join("core.sock")),
            deadline: Duration::from_secs(1),
        };
        let out = run(
            &[json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"commit","arguments":{"message":"x"}}})],
            &env,
        );
        assert_eq!(out[0]["result"]["isError"], true);
        assert!(out[0]["result"]["content"][0]["text"].as_str().unwrap().contains("orchestrator unavailable"));
    }

    #[test]
    fn forwards_to_the_core() {
        let d = tempfile::Builder::new().prefix("om").tempdir_in("/tmp").unwrap();
        let sock = d.path().join("core.sock");
        let l = orch_protocol::bind_listener(&sock).unwrap();
        let t = std::thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            let env: Envelope = bus::read_line(&mut BufReader::new(&s)).unwrap();
            let Msg::Mcp(m) = env.msg else { panic!() };
            assert_eq!((m.agent_id.as_str(), m.tool.as_str()), ("a1", "commit"));
            assert_eq!(m.arguments["message"], "fix: x");
            bus::write_line(
                &mut &s,
                &ReplyEnvelope::new(Reply::Mcp { is_error: false, text: "committed abc123".into(), data: Value::Null }),
            )
            .unwrap();
        });
        let env = McpEnv { agent_id: Some("a1".into()), socket: Some(sock), deadline: Duration::from_secs(5) };
        let out = run(
            &[json!({"jsonrpc":"2.0","id":"x","method":"tools/call","params":{"name":"commit","arguments":{"message":"fix: x"}}})],
            &env,
        );
        t.join().unwrap();
        assert_eq!(out[0]["id"], "x");
        assert_eq!(out[0]["result"]["isError"], false);
        assert_eq!(out[0]["result"]["content"][0]["text"], "committed abc123");
    }
}
