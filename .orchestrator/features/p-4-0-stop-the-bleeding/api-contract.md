# P4.0 — API contract (core ↔ UI)

> Companion of [spec.md](spec.md). Only what changes in P4.0. Back owns the Rust side, front the Svelte side; back lands first and front codes against this file.

## 1. Inbox items (`app.data.inbox.items`, built in `orch-app/src/inbox.rs`)

The item keeps its shape (`id, level, kind, agent_id, title, detail, preview, created_ms, actions, payload`) and gains:

| Field | Type | Meaning |
|---|---|---|
| `status` | `"open" \| "parked"` | Only open items are listed; `parked` = the hold expired, the agent's turn ended (§3.2). |
| `decided_by` | absent while open | Set on the outcome event (below), not on listed items. |

### `kind: "permission"` (store-backed, replaces the broker's `perm-N`)
`id` is the store inbox id (`inbox-<n>`, stable across restarts). Level 1.
```json
"payload": {
  "tool": "Bash",
  "tool_input": { "command": "pnpm add zod", "description": "…" },
  "cwd": "/Users/…/worktrees/feat/front-1",
  "tool_use_id": "toolu_…",
  "summary": "pnpm add zod",
  "description": "Add zod",
  "ask_rule": 12,            // present when an ask rule sent it; then no always_allow
  "always": "Bash(pnpm add:*)", // the rule "Always allow" would add, if offered
  "restarted": true           // only after a core restart
}
```
`actions`: open → `["a","r","enter","o","in_pane"]` (+ `"always"` when `payload.always` is set and `ask_rule` is absent); parked → `["a","r","enter","o"]` (+ `"always"` likewise). `enter` = deny with a message.

### `kind: "startup"` (new, FR-415)
Level 2. `title`: `"<agent> did not start"` or `"<agent> could not resume"`.
```json
"payload": { "waiting_on": "startup", "mode": "new" | "resume", "since_ms": 0,
             "failed": "resume" | "fresh",   // absent for "no SessionStart in 30 s"
             "exit_status": 1, "tail": "…ANSI-stripped PTY tail…" }
```
`actions`: `["o","restart_fresh","dismiss"]` (+ `"delete"` when `failed` is set). `preview` = last lines of `tail`.

### `kind: "gate"` after a restart
Original gates keep their payload and gain `restarted: true`. Legacy holds: `payload.action = "restart_gate"`, `{agent, detail}`; actions `["a","r","o"]`. `gate_detail` returns `{ action: "restart_gate", detail }` for it (no error).

### Removed
- `app.data.permissions` (broker list): the agent pane strip filters `inbox.items` by `kind === "permission" && agent_id === <agent>`.
- `wait-<agent>` is no longer produced for an agent whose only wait is a pending or parked permission item.

## 2. Commands

| Command | Change |
|---|---|
| `inbox_act { item, action, message? }` | Actions for `permission`: `approve` (allow), `reject` (deny, `message` optional), `always`, `in_pane` (open only). For `startup`: `restart_fresh`, `dismiss`, `delete`. Returns `{ ok, effect }` where `effect` is `"allowed" \| "denied" \| "resumed" \| "delivered" \| "queued" \| "restarted" \| "in_pane" \| "dismissed"` so the toast states what happened. |
| `restart_fresh { agent }` | New (agent menu). Same effect as the `startup` action. |
| `permission_answer` (broker) | **Removed**; use `inbox_act`. |

## 3. Events to the UI
The existing `inbox` change notification is unchanged. A decided item emits `inbox_outcome { item, decided_by, effect, at_ms }` (`decided_by`: `user | pane | rule:<id> | expired | core`); front uses it for the toast and to drop the card. No other push channel in P4.0.

## 4. ctl / orchd (no UI)
- `CtlRequest::RunReport { team_run_id?, since_ms?, until_ms? }` → `RunReport { agents: [{id, role, permission_mode}], prompts: { by_tool: {..}, by_decider: {..}, total }, interventions: { by_cause: {..}, total }, waiting_ms, startup_blocked, restarted_fresh, pressure_changes, pressure_suspends }`.
- `orchd report [--run <id>] [--since <iso>] [--until <iso>] [--json] [--baseline]`.
- `orchd audit` output gains the `actor` values `pane`, `expired`.
