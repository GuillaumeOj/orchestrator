//! `orch-mcp`: the orchestrator's MCP server, run by Claude Code over stdio
//! for each agent (SPEC §6, ADR-009 decision 5). It speaks JSON-RPC 2.0, one
//! message per line, and forwards each tool call to the core over the bus;
//! the agent's identity comes from the environment the core set at spawn.
//!
//! Tools (an agent sees the subset `ORCH_TOOLS` names; a solo session has
//! `commit` and `request_pr`):
//! - `commit(message, paths?)`: the core commits in the agent's worktree,
//!   outside the sandbox (sandboxed git can't write the repository's git dir
//!   under Claude Code 2.1.302), logged and subject to permission rules;
//! - `request_pr(title, body?, draft?)`: asks the user to push the branch and
//!   open a PR (an Inbox gate). Agents never push themselves;
//! - team tools (P2): `send_message`, `inbox`, `list_tasks`, `handoff`,
//!   `request_review`, `ask_user`, `create_task`, `update_task`,
//!   `request_spec_approval`, `merge`, `report_done`, `sync_integration`.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::time::Duration;

use orch_protocol::{CONNECT_TIMEOUT_MS, DEFAULT_MCP_DEADLINE_MS, Envelope, McpMsg, Msg, Reply, bus};
use serde_json::{Value, json};

/// Protocol revisions this server accepts; it answers with the client's when listed.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

#[derive(Clone, Debug, Default)]
pub struct McpEnv {
    pub agent_id: Option<String>,
    pub socket: Option<PathBuf>,
    pub deadline: Duration,
    /// The tools this agent may call (`ORCH_TOOLS`); empty = [`SOLO_TOOLS`].
    pub tools: Vec<String>,
}

impl McpEnv {
    pub fn from_env() -> McpEnv {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        McpEnv {
            agent_id: var(orch_protocol::env::AGENT_ID),
            socket: var(orch_protocol::env::SOCKET).map(PathBuf::from),
            deadline: Duration::from_millis(DEFAULT_MCP_DEADLINE_MS),
            tools: var(orch_protocol::env::TOOLS)
                .map(|v| v.split(',').map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect())
                .unwrap_or_default(),
        }
    }

    pub fn allowed(&self) -> Vec<String> {
        if self.tools.is_empty() { SOLO_TOOLS.iter().map(|s| s.to_string()).collect() } else { self.tools.clone() }
    }
}

/// Tools a solo session has when `ORCH_TOOLS` is not set (P1).
pub const SOLO_TOOLS: &[&str] = &["commit", "request_pr"];

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {"type": "object", "properties": properties, "required": required, "additionalProperties": false}
    })
}

/// Every orchestrator tool (names are what Claude Code shows as
/// `mcp__orch__<name>`). An agent sees the subset its role allows.
pub fn tools() -> Value {
    let s = |d: &str| json!({"type": "string", "description": d});
    Value::Array(vec![
        tool(
            "commit",
            "Commit changes in your worktree. Use this instead of `git commit`: the sandbox blocks git from writing the repository. Stages the given paths (or every change when `paths` is omitted, like `git add -A`) and commits them on your branch. Hooks do not run. It never pushes. During a merge started by sync_integration, it concludes the merge.",
            json!({
                "message": s("The commit message (first line = summary)."),
                "paths": {"type": "array", "items": {"type": "string"}, "description": "Paths relative to the worktree root. Omit to commit everything."}
            }),
            &["message"],
        ),
        tool(
            "request_pr",
            "Ask the user to push your branch and open a pull request. You cannot push yourself. This creates an approval item for the user and returns at once; you are told the result later.",
            json!({"title": {"type": "string"}, "body": {"type": "string"}, "draft": {"type": "boolean"}}),
            &["title"],
        ),
        tool(
            "send_message",
            "Send a message to another node of your team (by node id, e.g. `lead`, `back`), or to one agent by its full id (e.g. `<feature>-back-1`). Only the recipients the team rules allow. Returns at once; replies arrive as ORCH: notes.",
            json!({"to": s("Node id or agent id."), "body": s("The message.")}),
            &["to", "body"],
        ),
        tool(
            "inbox",
            "List your unread messages (and mark them read). New messages already reach you by themselves, as a turn or as an ORCH: note: call this only to read them again or to see older ones with `all`.",
            json!({"all": {"type": "boolean", "description": "Also list messages already read."}}),
            &[],
        ),
        tool(
            "list_tasks",
            "List the tasks of this feature with their id, status, assignee node, dependencies and branch.",
            json!({"mine": {"type": "boolean", "description": "Only the tasks assigned to your node."}, "status": s("Only this status: backlog, ready, in_progress, review, done.")}),
            &[],
        ),
        tool(
            "handoff",
            "Hand an artifact (a file in your worktree, a design link, a build) to another node. It is attached to the feature and the recipient is told. Designs: export each frame as an image under the feature's design/ folder (.orchestrator/features/<slug>/design/<name>.png or .svg), commit it, and hand off that repo path with kind \"design\" (the frame link goes in the note): agents can't open a link-only design.",
            json!({"artifact": s("A repo-relative path in your worktree (committed) or a URL."), "to": s("Node id."), "kind": s("doc, design or build."), "note": s("What it is for; for a design, the Penpot/Figma frame link.")}),
            &["artifact", "to"],
        ),
        tool(
            "request_review",
            "Ask a node to review a branch (yours by default). The reviewer replies with send_message.",
            json!({"to": s("Node id."), "branch": s("Branch to review; default: your own."), "note": s("What to look at.")}),
            &["to"],
        ),
        tool(
            "ask_user",
            "Ask the user a question when your team can't answer it: the only way to reach the user (a question written in your reply is not read). Returns at once; the answer arrives later as a new turn.",
            json!({
                "question": s("The question, with the context the user needs to answer it."),
                "options": {"type": "array", "items": {"type": "string"}, "description": "Suggested answers."},
                "kind": s("decision (default: the user is notified) or fyi (shown in the Inbox, no notification).")
            }),
            &["question"],
        ),
        tool(
            "create_task",
            "Create a task of this feature and assign it to a node. Only after the spec gate is approved. A task starts by itself once its dependencies are done.",
            json!({
                "title": s("Short title."),
                "body": s("What to do, and how to know it is done."),
                "node": s("The node that does it."),
                "depends_on": {"type": "array", "items": {"type": "integer"}, "description": "Ids of tasks that must be done first."},
                "status": s("ready (default) or backlog."),
                "plan_first": {"type": "boolean", "description": "The owner sends you a plan (submit_plan) and may not edit until you approve it (decide_plan). For large or unclear tasks, or ones that touch a shared contract."}
            }),
            &["title", "node"],
        ),
        tool(
            "update_task",
            "Change a task's status (backlog, ready, in_progress, review, done) or add a note. Specialists only update their own tasks.",
            json!({"id": {"type": "integer"}, "status": s("New status."), "notes": s("A note added to the task."), "node": s("Reassign to this node (lead only)."), "plan_first": {"type": "boolean", "description": "Ask for a plan before any edit (lead only)."}}),
            &["id"],
        ),
        tool(
            "request_spec_approval",
            "Ask the user to approve the feature docs (spec gate). Commit the docs first. Returns at once; end your turn, you are told the decision.",
            json!({"summary": s("What the spec decides and the tasks you plan.")}),
            &["summary"],
        ),
        tool(
            "merge",
            "Merge a branch of this feature (e.g. orch/<feature>/back-1) into the integration branch, in your worktree. On a conflict nothing is merged and the branch's owner is told to sync and fix.",
            json!({"branch": s("The branch to merge."), "task": {"type": "integer", "description": "The task it completes (set to done)."}}),
            &["branch"],
        ),
        tool(
            "report_done",
            "Report your current task done to the node you report to (it moves to review). Commit everything first. When the orchestrator had you review your own branch, call it again with `review`.",
            json!({
                "summary": s("What you did and how it was tested."),
                "task": {"type": "integer", "description": "Task id; default: your current task."},
                "review": {
                    "type": "object",
                    "description": "After your self-review: what you fixed, and each finding you left with why.",
                    "properties": {
                        "fixed": {"type": "array", "items": {"type": "string"}},
                        "unfixed": {"type": "array", "items": {"type": "object", "properties": {"finding": {"type": "string"}, "reason": {"type": "string"}}, "required": ["finding"]}}
                    }
                }
            }),
            &["summary"],
        ),
        tool(
            "team_status",
            "The state of every member of your team: running, idle, waiting for the user, suspended or queued (and why), with its task.",
            json!({}),
            &[],
        ),
        tool(
            "nudge",
            "Tell a member of your team what to do next (by agent or node id). It gets it as its next turn: use it to unblock a member that stopped, to answer what it asked, or to send a review back.",
            json!({"agent": s("Agent or node id."), "text": s("What it should do.")}),
            &["agent", "text"],
        ),
        tool(
            "resume_agent",
            "Restart a suspended or failed member of your team (by agent or node id) so it carries on with its task. It starts when the orchestrator admits it.",
            json!({"agent": s("Agent or node id.")}),
            &["agent"],
        ),
        tool(
            "decide_plan",
            "Approve the plan a member sent for a plan-first task (its edits are then allowed), or send it back with what to change.",
            json!({"task": {"type": "integer"}, "approve": {"type": "boolean"}, "note": s("What to change, or what to keep in mind.")}),
            &["task", "approve"],
        ),
        tool(
            "sync_integration",
            "Merge the latest integration branch into your branch. On a conflict the merge stays in progress: fix the conflicted files, then call commit.",
            json!({}),
            &[],
        ),
        tool(
            "submit_plan",
            "Send your plan for a plan-first task to the node you report to. Returns at once: end your turn; the decision comes as a new turn.",
            json!({"plan": s("What you will change, file by file, and how you will test it."), "task": {"type": "integer", "description": "Task id; default: your current task."}}),
            &["plan"],
        ),
        tool(
            "submit_review",
            "Send the findings of a review you were asked for. They go to the branch's owner and to the lead.",
            json!({
                "task": {"type": "integer"},
                "findings": {"type": "array", "items": {"type": "string"}, "description": "One finding each: file:line, the problem, the fix. Empty when the branch is fine."},
                "summary": s("Your verdict in a sentence.")
            }),
            &["task", "findings"],
        ),
    ])
}

/// The tools an agent allowed `allowed` sees, in [`tools`] order.
pub fn tools_for(allowed: &[String]) -> Value {
    Value::Array(
        tools()
            .as_array()
            .into_iter()
            .flatten()
            .filter(|t| allowed.iter().any(|a| t["name"] == a.as_str()))
            .cloned()
            .collect(),
    )
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
pub fn handle(msg: &Value, allowed: &[String], call: &dyn Fn(&str, &Value) -> (bool, String)) -> Option<Value> {
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
                    "instructions": "Orchestrator tools. Commit with `commit` (git commit is blocked by the sandbox). In a team, talk to the other nodes with `send_message` and read your messages with `inbox`."
                }),
            )
        }
        "ping" => result(&id, json!({})),
        "tools/list" => result(&id, json!({"tools": tools_for(allowed)})),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            if !tools_for(allowed).as_array().is_some_and(|t| t.iter().any(|x| x["name"] == name)) {
                return Some(error(&id, -32602, &format!("unknown tool {name:?}")));
            }
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            let (is_error, text) = call(name, &args);
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
    let Some(agent_id) = &env.agent_id else {
        return (true, "orchestrator unavailable (ORCH_AGENT_ID not set)".into());
    };
    let r = (|| -> Result<(bool, String), String> {
        let m = Envelope::new(Msg::Mcp(McpMsg {
            agent_id: agent_id.clone(),
            tool: tool.into(),
            arguments: args.clone(),
            deadline_ms: env.deadline.as_millis() as u64,
        }));
        let reply = bus::round_trip(socket, &m, Duration::from_millis(CONNECT_TIMEOUT_MS), Some(env.deadline))
            .map_err(|e| e.tag())?;
        match reply {
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
    let allowed = env.allowed();
    for line in input.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(Value::Array(batch)) => {
                let rs: Vec<Value> = batch.iter().filter_map(|m| handle(m, &allowed, &call)).collect();
                if rs.is_empty() { None } else { Some(Value::Array(rs)) }
            }
            Ok(m) => handle(&m, &allowed, &call),
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
    use orch_protocol::ReplyEnvelope;
    use std::io::{BufReader, Cursor};

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
    fn a_team_agent_sees_only_its_tools() {
        let env = McpEnv { tools: vec!["send_message".into(), "inbox".into()], ..Default::default() };
        let out = run(
            &[
                json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
                json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"merge","arguments":{"branch":"x"}}}),
            ],
            &env,
        );
        let names: Vec<_> = out[0]["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].clone()).collect();
        assert_eq!(names, ["send_message", "inbox"]);
        assert_eq!(out[1]["error"]["code"], -32602, "a tool the role lacks is unknown");
        for t in tools().as_array().unwrap() {
            assert_eq!(t["inputSchema"]["type"], "object", "{}", t["name"]);
        }
    }

    #[test]
    fn core_down_is_a_tool_error_not_a_crash() {
        let d = tempfile::Builder::new().prefix("om").tempdir_in(orch_protocol::test_tmp()).unwrap();
        let env = McpEnv {
            agent_id: Some("a1".into()),
            socket: Some(d.path().join("core.sock")),
            deadline: Duration::from_secs(1),
            tools: vec![],
        };
        let out = run(
            &[
                json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"commit","arguments":{"message":"x"}}}),
            ],
            &env,
        );
        assert_eq!(out[0]["result"]["isError"], true);
        assert!(out[0]["result"]["content"][0]["text"].as_str().unwrap().contains("orchestrator unavailable"));
    }

    #[test]
    fn forwards_to_the_core() {
        let d = tempfile::Builder::new().prefix("om").tempdir_in(orch_protocol::test_tmp()).unwrap();
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
        let env =
            McpEnv { agent_id: Some("a1".into()), socket: Some(sock), deadline: Duration::from_secs(5), tools: vec![] };
        let out = run(
            &[
                json!({"jsonrpc":"2.0","id":"x","method":"tools/call","params":{"name":"commit","arguments":{"message":"fix: x"}}}),
            ],
            &env,
        );
        t.join().unwrap();
        assert_eq!(out[0]["id"], "x");
        assert_eq!(out[0]["result"]["isError"], false);
        assert_eq!(out[0]["result"]["content"][0]["text"], "committed abc123");
    }
}
