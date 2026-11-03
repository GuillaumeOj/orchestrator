# P4.4 — API contract

> Companion of [spec.md](spec.md). Back owns the Rust side (policy, Overwatch runtime and tools), front the Svelte side; front may mock §3–4 until back lands them. Field names are final; adding optional fields later is allowed, renaming is not. P4.1's contract (Decisions, `now`) and P4.3's (ChatEntry, `chat_*` commands, `focus_card`) stand.

## 1. Policy file (`policy.toml`)

```toml
# <app data>/policy.toml (global) · <project>/.orchestrator/policy.toml or <app data>/projects/<id>/policy.toml
version = 1

[defaults]                 # global file only; also editable in Settings → Permissions
team = "overwatch"         # effect when no rule matches, team roles   (overwatch | user)
solo = "user"              # effect when no rule matches, solo sessions (overwatch | user)

[[rule]]
id = "installs"            # optional; label in lines ("policy: installs"); unique per file; generated "r<n>" if absent
match = "Bash(cargo add:*)"  # FR-708 grammar: Tool | Tool(spec:*) | Tool(spec*) | Tool(dir/**) | Tool(exact)
effect = "allow"           # allow | deny | overwatch | user   ("ask" read as "user")
role = "back"              # optional: team role name, or "solo"
agent = "back-1"           # optional (migrated agent-scoped rules)
note = "routine installs"  # optional
# written by the core: from = "inbox-42" (Always allow) or "proposal:inbox-97"; added_ms = 1793600000000
```
- Project rules apply to that project's agents only; global rules to all. Matching rules from both files are merged; the most restrictive effect wins: `deny` > `user` > `overwatch` > `allow`.
- The core writes with `toml_edit`: appends `[[rule]]` tables, removes one by `id`, never reorders or drops comments.
- A rule refused by the hard lists (an `allow` / `overwatch` that would cover a hard-deny or user-only call) is loaded but has no effect on those calls; Settings shows a warning on it.

## 2. Core ↔ Overwatch

### 2.1 Events (what a turn carries)

The turn text is a header line and one line per event; long batches go in a file (FR-206), the turn saying "Read messages/<file>".
```jsonc
// Event
{
  "id": "ev-311",
  "at_ms": 1793600000000,
  "kind": "permission",       // permission | question | stuck | user_message | rule_candidate   (blocking)
                              // | decision_open | decision_closed | outcome                       (batched)
  "decision": "inbox-42",     // permission, question, rule_candidate (the 3rd), decision_open/closed
  "agent": "back-1",          // permission, question, stuck
  "run": "run-12",            // when the agent is in a run
  "summary": "Bash: pnpm add zod · cwd shop-api/back-1 · effect overwatch · 60 s",
  "deadline_ms": 1793600060000, // permission: until when decide_permission is accepted
  "stuck": { "why": "idle with T-4 for 12 min", "since_ms": 0 },    // stuck
  "text": "why is checkout slow today?",                           // user_message
  "chat": "c-830",                                                  // user_message: its entry
  "candidate": { "match": "Bash(pnpm add:*)", "effect": "allow", "scope": "project", "project": "shop-api", "role": "front",
                 "from": ["inbox-12", "inbox-30", "inbox-42"] }     // rule_candidate
}
```

### 2.2 Tools (`mcp__orch__*`, Overwatch only)

| Tool | Args | Returns |
|---|---|---|
| `world` | `{ project? }` | the `now_snapshot` attention part: `{ agents: [AgentAttention], runs: [RunAttention], counts, decisions: [open Decision, payload omitted], queue: [QueueEntry], governor: { week_pct, plan_pct, reserve_left_pct, last_actions: [ {at_ms, action, agent, why} ×20 ] }, overwatch: { status } }` |
| `inspect` | `{ agent }` | `{ attention, task: {id, title, status, body}?, run?, messages: [last 10 team messages], transcript_tail: [last 10 assistant texts, ≤ 400 chars each], decisions: [open ones of this agent], worktree, feature_docs: [paths] }` |
| `decide_permission` | `{ decision, effect: "allow" \| "deny", reason }` | `{ ok: true, line }` · errors `not_allowed` (hard / user-only / effect is not `overwatch`), `too_late`, `stale` |
| `answer_question` | `{ decision, answers: [ { question_index?, text } ], sources: [ "docs/…md#3", ".orchestrator/features/x/api-contract.md#2", "chat:c-790" ] }` | `{ ok: true, line }` · `ungrounded` (no or invalid source), `stale`, `bad_request` |
| `annotate` | `{ decision, note, recommend?: action_id, sources? }` | `{ ok: true }` (replaces an earlier one) · `stale` |
| `nudge` | `{ agent, text }` | `{ ok: true }` |
| `resume_agent` | `{ agent }` | `{ ok: true, how: "resumed" \| "queued" \| "restarted_fresh" }` |
| `restart_fresh` | `{ agent, why }` | `{ ok: true }` |
| `reprioritize` | `{ entry, tier: "low" \| "normal" \| "high", why }` | `{ ok: true, from }` · `not_allowed` (above the allocation's tier) |
| `propose_rule` | `{ match, effect, scope: "project" \| "global", project?, role?, from: [decision ids], why }` | `{ ok: true, decision }` · `not_allowed`, `duplicate` (one open per kind of call) |
| `post` | `{ text, thread? }` | `{ ok: true, entry }` |

Errors: `{ code, message }`. Every acting call is audited with actor `overwatch`.

## 3. Core ↔ UI

### 3.1 `now` additions
```jsonc
{
  …,
  "overwatch": {                       // always present
    "status": "working",               // off | asleep | waking | working | paused | down
    "events": 2,                       // queued + in the running turn
    "why": "login expired",            // down, paused
    "since_ms": 1793600000000,
    "agent": "overwatch"               // to open its session
  }
}
// Decision gains (optional):
"annotation": { "note": "Not in the allow list; I'd allow it for this project.", "recommend": "always", "sources": ["chat:c-790"], "at_ms": 0 },
"deciding_until_ms": 1793600060000,    // permission with effect overwatch, while the Overwatch may still settle it
"waits_on": "overwatch"                // during deciding_until_ms, then "user" (badge and notifications follow waits_on = user, unchanged)
// "topics" gains "overwatch" when the status changes
```
Decision kind `rule_proposal` (level 4, `waits_on: "user"`, not counted in `counts.decisions`): context blocks `markdown` (the text), `rule` (new block: `{ match, effect, scope, project?, role?, file }`), `links` (new block: `[ { decision, label, chat } ]`); actions `add` (a), `not_now` (r), `edit` (⏎).

### 3.2 ChatEntry additions (P4.3 §1 shape)
- `author: "user"` / `"overwatch"`, `kind: "text"`: `text` (markdown), `thread?`, `delivery?: "waking" | "pending" | "delivered"` (user entries: pending while the Overwatch is down / off), `reply_to?` (an Overwatch text answering a user entry: that entry's id).
- `kind: "annotation"` is **not** used as a separate entry: the annotation lives on the decision and its card snapshot (D7).
- New `kind: "action"` (author overwatch): an unsticking / reprioritize action, with `line { verb: "nudged" | "resumed" | "restarted" | "reprioritized", agent, what, by: "overwatch", by_detail: reason, state: "effect", at_ms }`.
- `kind: "rule_proposal"`: a decision entry for the `rule_proposal` decision (same as `decision`, kept as its own kind for search and the "Not now" history).
- `line` gains: `sources?: [string]` (answered / denied / allowed by the Overwatch), `undo?: { id: "u-…", label: "Revoke rule" | "Tell the agent" | "Allow instead" | "Correct the answer" | "Restore tier" | "Remove rule", until?: "condition" }`, `undone_at_ms?`, `undo_gone?: "back-1 already moved on"`. Line verbs gain `nudged`, `resumed`, `reprioritized`, `added` (rule).
- `by: "policy"` replaces `rule` for policy-file answers (`by_detail` = the rule's `id`); `rule` stays readable on old entries.

### 3.3 Commands (Tauri; `orchd` server method of the same name)

| Command | Args | Returns |
|---|---|---|
| `chat_send` | `{ text, thread? }` | `{ entry }` — writes a `user` text entry, emits a `user_message` event |
| `chat_undo` | `{ entry, text? }` | `{ ok: true, entry }` (the line updated) · `gone` with the reason · `text` required for "Correct the answer" |
| `overwatch_restart` | `{}` | `{ ok: true }` — fresh session, new briefing |
| `overwatch_settings` / `set_overwatch_settings` | `{ enabled, model, batch_secs, permission_secs, idle_secs, rule_threshold }` | the settings |
| `policy_files` | `{}` | `[ { scope: "global" \| "project", project?, path, rules, error?: {line, message}, warnings: [ {id, message} ] } ]` |
| `policy_defaults` / `set_policy_defaults` | `{ team, solo }` | the defaults |
| `inbox_act` (P4.1) | action ids `add`, `not_now`, `edit` for `rule_proposal` | as before |

`open_in_editor` (FR-1101, existing) opens a policy file path.

## 4. `orchd`
- `orchd overwatch`: status, today's session id, turns `09:12:04 resume warm events=3 wake=6.2s first_tool=8.1s 41s`, p50 / p90 wake.
- `orchd policy [--project P] [--test "Bash" "pnpm add zod" --role front]`: the merged rules, and for `--test` the effect and the rule (or hard list entry) that decides it.
