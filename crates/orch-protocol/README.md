# orch-protocol

Versioned messages and socket helpers for Orchestrator's **local bus**: the Unix socket between the
Rust core and the helpers Claude Code launches (`orch-hook`, `orch-status`, later `orch-mcp`), and
the control clients (`orchd` now, the Tauri UI later). SPEC §6, FR-806, NFR-3, ADR-009 decision 5.

## Wire format

- One connection per request. The client writes **one JSON line**, the core answers **one JSON
  line** and closes. The only exception is `ctl` `attach`, after which the connection carries raw
  PTY bytes both ways.
- Every request is an `Envelope` `{"v": 1, "type": "hook" | "status" | "ctl" | "mcp", …}`. A peer that sees
  another `v` answers `{"reply":"error"}`; the helpers then behave as if the core were down.
- Replies: `{"reply":"hook","output":<json|null>}`, `{"reply":"ack"}`, `{"reply":"ctl","ok":…,"data":…}`,
  `{"reply":"mcp","is_error":…,"text":…,"data":…}`, `{"reply":"error","message":…}`.

| Type | Sender | Content |
|---|---|---|
| `hook` | orch-hook | `agent_id`, `event` (parsed `hook_event_name`), `ts_ms`, `deadline_ms` (0 = not waiting), raw `payload` |
| `status` | orch-status | `agent_id`, `ts_ms`, raw statusline `payload` |
| `ctl` | orchd / UI | `CtlRequest`: agents (`create_worktree`, `start`, `resume`, `suspend`, `kill`, `interrupt`, `deliver`, `attach`, `resize`, `read`, `state`, `usage`, `governor`, `pause_all`, `resume_all`, `shutdown`); Inbox and rules (`inbox`, `decide` with `InboxDecision`, `rules`, `add_rule`, `revoke_rule`, `audit`, `events`); projects and git (`project_add`, `project_clone`, `projects`, `project`, `fetch`, `diff`, `remove_worktree`, `removal_plan`, `remove_project`, `relocate_project`, `commit`, `search`) |
| `mcp` | orch-mcp | `McpMsg`: `agent_id`, `tool` (`commit`, `request_pr`), `arguments`, `deadline_ms` (300 s by default); answered with `Reply::Mcp` |

## Socket

- `bind_listener` checks the path is under macOS's 104-byte `sun_path` limit, removes a **stale**
  socket file (nobody answers on it, as after `kill -9`), refuses to take over a live one, and sets
  **0600**. A missing parent folder is created 0700.
- `connect(path, timeout)` uses a non-blocking connect with a deadline (≤ 1 s for helpers, FR-806).
  A missing file is `NotFound`, a stale socket `Refused`; both fail at once.

## Hooks

- `HookEvent` keeps unknown names (`Other`), so a newer CLI never breaks parsing.
  `waits_for_core()` is true only for PreToolUse and PermissionRequest.
- `HookPayload::parse` reads the fields the core uses (prompt_id, background_tasks, tool fields,
  StopFailure `error`, subagent fields…) and never fails.
- `HookOutput` builds only the shapes verified in P0: PreToolUse `permissionDecision`,
  PermissionRequest `decision.behavior`, `additionalContext` (always prefixed `ORCH:`), Stop
  `{"decision":"block"}`. `fail_closed(event)` is deny for the two tool events and nothing for the
  others.

## Statusline

`StatusSample::parse` reads `rate_limits.{five_hour,seven_day}`, `cost.total_cost_usd`,
`context_window`, `prompt_cache` and `model`. A window missing from the response stays `None`
(unknown, not 0).

## Spool

`<agent dir>/spool.jsonl`, one `SpoolRecord` per line (`v`, `ts_ms`, `agent_id`, `event`, `why`,
`payload`). Appended with one `O_APPEND` write per record; torn lines are skipped and counted when
read back. The reconciler replays and then renames the file.

## Admission types

`AdmissionRequest` / `AdmissionDecision` are the Governor's decisions as the core logs and reports
them (enforced verdict, proposed verdict for shadow mode, reason, estimated start).

## Readings

- The spec says "versioned message types" without a format. This crate uses an integer `v` on
  every request and one JSON line per message, the format the P0 recovery harness used.
