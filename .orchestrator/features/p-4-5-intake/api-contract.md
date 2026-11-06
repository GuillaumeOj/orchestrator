# P4.5 — API contract

> Companion of [spec.md](spec.md). Field names are final; back implements them, front consumes them (mocks from §2 until back lands). JSON is snake_case. Times are `*_ms` (epoch ms). Additive only: no existing field changes meaning.

## 1. Config and items (core)

### 1.1 `IntakeSource` (`projects.intake_source`, migration 17; `null` = none)
```jsonc
{ "type": "none" }
{ "type": "github", "repo": "acme/shop" /* optional, default known_repo */,
  "filter": { "kind": "label", "names": ["ready"] } }
//          { "kind": "milestone", "title": "v2.1" }
//          { "kind": "project", "owner": "acme", "number": 7, "status": "Todo" /* optional */ }
{ "type": "linear", "team": "<team id>", "project": "<project id>" /* opt */, "label": "next" /* opt */, "include_backlog": false }
{ "type": "repo_docs", "glob": ".orchestrator/backlog/*.md", "branch": null /* null = base branch */ }
```
Validation errors (`intake_source_set`, `intake_preview`): `{ code: "bad_request", field, message }` — `repo` not `owner/name`, empty `names`, `number` < 1, Linear `team` empty, glob that does not parse or starts with `/` or contains `..`.

### 1.2 `IntakeItem`
```jsonc
{
  "id": "it_41",                 // stable per (project, source, ref)
  "project": "p_3",
  "source": "github",            // github | linear | repo_docs
  "ref": "#123",                 // display ref: "#123" | "ENG-42" | ".orchestrator/backlog/checkout-v2.md"
  "title": "Refunds on partial orders",
  "url": "https://github.com/acme/shop/issues/123", // repo_docs: null (opened in the editor)
  "labels": ["ready", "size:M"],
  "updated_at_ms": 1762400000000,
  "size": "M",                   // S | M | L | null (unknown → estimate uses M)
  "mode": "team",                // suggested: team | solo
  "team": "default-web",         // suggested team id for mode team, or null
  "estimate": { /* §2.2 */ },
  "state": "ready",              // ready | started | hidden (only ready and hidden reach `now`)
  "excerpt": "First 200 chars of the body, plain text"
}
```
The full body (markdown, ≤ 4 000 chars, `truncated: bool`) comes from `intake_item(item)` only.

## 2. `now` (core → UI)

### 2.1 Snapshot and delta
`now_snapshot` gains `intake`; a `now` delta carries `intake` (whole, replaces) only when it changed.
```jsonc
"intake": {
  "items": [ IntakeItem, … ],      // state ready and hidden; hidden ones are counted, rendered on "Show"
  "sources": [
    { "project": "p_3", "type": "github", "label": "acme/shop · label ready",
      "status": "ok",              // ok | refreshing | error | none
      "error": null,               // { code, message, fix } when status = error; fix: "settings" | "integrations" | "gh_scope"
      "refreshed_at_ms": 1762400000000,
      "count": 4, "more": 0, "hidden": 1 }
  ]
}
```
Every registered project has one `sources` row (`status: none` and `type: none` without a source). Error codes: `gh_missing`, `gh_auth`, `gh_scope`, `not_found`, `linear_key`, `linear_auth`, `rate_limited` (message has the retry time), `bad_glob`, `branch_missing`, `failed`.

### 2.2 `Estimate`
```jsonc
"estimate": {
  "team": { "low_pp": 4.1, "high_pp": 6.8, "basis": "project", "samples": 7 },
  "solo": { "low_pp": 0.9, "high_pp": 1.6, "basis": "global",  "samples": 12 },
  "size": "M",
  "room_pp": 3.0,                  // what is left this week for the project's allocation (or the week when unplanned); null when unknown
  "over_room": true                // high_pp of the suggested mode > room_pp
}
```
`basis`: `project` | `global` | `default`. Values rounded to tenths.

### 2.3 Origin on rows
`AgentAttention.origin` and `RunAttention.origin` (optional, absent when not from intake):
```jsonc
"origin": { "kind": "intake", "item": "it_41", "source": "github", "ref": "#123", "url": "https://…" }
//        { "kind": "brief", "decision": "d_88" }
```

## 3. Commands (UI → core, Tauri)

### 3.1 Intake
| Command | Params | Returns |
|---|---|---|
| `intake_source` | `project` | `{ source: IntakeSource \| null, default_glob, default_branch, known_repo }` |
| `intake_source_set` | `project, source` | `{ preview }` (as `intake_preview`), after storing and refreshing |
| `intake_preview` | `project, source` | `{ count, more, items: IntakeItem[≤5], error }` — nothing stored |
| `intake_options` | `project, source` | GitHub: `{ labels[], milestones[{title, open_issues}] }`; Linear: `{ labels[] }` of the team; repo docs: `{ matches }` (count for the glob) |
| `intake_refresh` | `project?` (all when absent) | `{}`; the result arrives in `now` |
| `intake_item` | `item` | `{ item: IntakeItem, body, truncated }` |
| `intake_hide` / `intake_unhide` | `item` | `{}` |

### 3.2 Starting from an item or a brief
- `team_launch(form)`: `LaunchSpec` gains optional `intake: { item?: string, brief_decision?: string }` (at most one set).
- `agent_start(req)`: `NewSession` gains the same optional `intake`.
- The core checks the item is `ready` (else `{ code: "stale", message: "already started" }`) and, for a brief, that the decision is open. On success: item `started`, event `intake.started`, or brief decision closed with effect `launched <slug | agent>` and event `intake.planned_started`.
- `intake_prefill(item | brief_decision, mode)` → `{ launch: LaunchSpec-like {project_id, team_id, slug, title, brief, base:null, off_hours:false} }` or `{ session: NewSession-like {project_id, name, message} }`: the core builds the prefill (slug unique, brief text, team), so the UI and the brief card use one rule.

### 3.3 Chat
`chat_send(text, thread?, context?)`: `context = { plan: "<project id>" }` marks the message as planning; the Overwatch's `user_message` event carries `plan: { project, name, path, docs_dir, intake: <source label or null>, recent_briefs: [title ×≤5] }`.

## 4. Overwatch tool (core ↔ Overwatch)
`propose_brief`
```jsonc
{ "project": "p_3", "title": "Partial refunds", "slug": "partial-refunds" /* optional */,
  "brief": "markdown, ≤ 8000 chars", "mode": "team" /* team | solo */, "team": "default-web" /* optional */,
  "reply_to": "<chat entry id of the user's message>" /* optional, threads the card */ }
→ { "decision": "d_88" }
```
Errors: `bad_request` (unknown project / team, empty or too long title or brief, slug taken by an open feature), `not_allowed` (the Overwatch is not the caller). A second call in the same thread with an open `brief` decision closes the first (`effect: "revised"`) and returns the new id. Declared in `orch-mcp` and `OVERWATCH_TOOLS` only.

## 5. Decision kind `brief`
- Level 4 (no notification, not in the badge, `waits_on: user` but excluded from needs-you counts like `rule_proposal`).
- Context blocks: `markdown` (the brief), `text` (project · mode · team), `cost` (the Estimate of §2.2, size M).
- Actions: `{id: "launch", label: "Launch", key: "⏎"}`, `{id: "edit", label: "Edit", key: "e"}`, `{id: "dismiss", label: "Dismiss", key: "r"}`. `launch` and `edit` are **open** actions (no decide): `inbox_act` → `routeOpen { open: "launch", prefill }` (new `routeOpen` target opening the `launch` or `new-session` overlay with `intake_prefill`'s result and `intake.brief_decision`); `dismiss` decides with effect `dismissed`.
- Lifecycle: open → launched `<slug>` (closed by the start) | dismissed | revised | stale (project removed).
- Chat entry kind: `decision` (as every card); its line verb `launched` / `dismissed` / `revised`, by `user` (or `overwatch` for revised).

## 6. Events and report
- `intake.refreshed {project, source, count, ms}`, `intake.error {project, code}`, `intake.started {item, project, source, mode, est_pp}`, `intake.planned_started {decision, project, mode}`, `intake.untracked_start {project, kind, ref}`, `intake.returned {item, why: run_deleted | session_killed}`.
- Weekly report JSON gains `intake: { started, planned, untracked }`.
- `orchd intake [--week] [--json]`.
