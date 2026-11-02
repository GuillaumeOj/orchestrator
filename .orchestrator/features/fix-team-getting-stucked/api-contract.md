# Fix Team Getting Stuck · API contract (core ↔ UI)

> Companion of [spec.md](spec.md). Only what changes. Back owns the Rust side, front owns the Svelte side. Back lands first (task 3), and front codes against this file. Names below are binding; if back has to change one, back updates this file in the same commit and tells front.

## 1. Blockers (one vocabulary for cards, chips, report and Lead notes)

`blocker` is a string, one of:

| value | meaning | UI label |
|---|---|---|
| `not_idle:<state>` | live agent in `<state>` (working, waiting_for_you, error, blocked_by_limit…) | "busy (<state>)" |
| `starting_up` | spawned, no SessionStart yet | "starting" |
| `no_ack` | pasted, the CLI did not acknowledge it (§3.1) | "paste not acknowledged" |
| `governor:<reason>` | held by the Governor (`off_hours`, `five_hour`, `limit`, `paused`, `model_switch`) | "held: <reason>" |
| `slot` | waiting for a live slot | "waiting for a slot" |
| `parked` | parked on a permission decision | "parked on your answer" |
| `crashed` | process gone, restart pending or exhausted | "crashed" |
| `user_typing` | the user is typing in the pane | "you are typing" |

## 2. Decision kind `delivery` (new)

Raised by the paste watchdog after 3 attempts (§3.1), by the delivery invariant (§3.7) and by Lead supervision (§3.7). Level 2. One open item per (agent, episode).

`title`:
- `"<agent> did not receive <n> message(s)"`
- or, for stuck episodes, `"<agent> looks stuck"`

```json
"payload": {
  "waiting_on": "delivery",
  "agent": "fix-x-back-1",
  "blocker": "no_ack",
  "since_ms": 0,
  "messages": [ { "id": 41, "from": "lead", "kind": "message", "preview": "first 120 chars", "attempts": 3 } ],
  "episode": "delivery" | "stuck_lead" | "stuck_wait" | "working_silent",
  "tail": "…ANSI-stripped PTY tail, last ~40 lines…"
}
```

`actions`: `["o", "retry", "restart_fresh", "dismiss"]`
- `o`: open the agent's pane.
- `retry`: re-queue the failed turn now, with `attempts` reset.
- `restart_fresh`: the existing FR-415 action.
- `dismiss`: close the item. The messages stay `failed`.

The item closes by itself (`decided_by: core`, `"delivered"`) once every listed message is `seen`.

The `startup` kind gains `failed: "crash"` (§3.5), with the same actions as `failed: "resume"`.

## 3. Team view (`team_admin.rs` feature view → `FeaturePane`)

Timeline events of kind `message`, `handoff`, `review_request` and `report` gain `delivery`:

```json
"delivery": { "state": "queued" | "handed" | "seen" | "read" | "failed",
              "attempts": 1, "via": "turn" | "inject" | null,
              "blocker": "slot" | null, "seen_at_ms": null, "read_at_ms": null }
```

New timeline event kinds (filter "Delivery" in `TL_FILTERS`):

| kind | detail |
|---|---|
| `redelivered` | `{agent, messages: [id], attempt, why: blocker}` |
| `delivery_failed` | `{agent, messages: [id], decision}` |
| `recovered` | `{agent, from_state, how: "continue" \| "resume" \| "fresh" \| "esc" \| "enter" \| "late_hook"}` |
| `stuck` | `{agent, episode}` |
| `tick_panic` | `{run, step, message}` |

## 4. Attention record (agents, mission control)

Each agent record gains:

```json
"undelivered": { "count": 2, "oldest_ms": 0, "blocker": "governor:off_hours" }
```

The field is absent when nothing has waited more than 60 s. `waiting_on.kind` gains `"delivery"`, set when the agent's top attention reason is a `delivery` decision. Its ref is the decision id, so `WaitingChip` opens it like `user`.

## 5. Run report (`ReportPane`, `orchd report`)

The report gains a `delivery` section:

```json
"delivery": {
  "sent": 0, "seen": 0, "read": 0, "failed": 0,
  "latency_ms": { "p50": 0, "p95": 0 },
  "redelivered": { "no_ack": 0, "not_idle": 0, "...": 0 },
  "recovered":   { "error": 0, "blocked_by_limit": 0, "crashed": 0, "working_silent": 0 },
  "stuck":       { "stuck_lead": 0, "stuck_wait": 0, "working_silent": 0 },
  "tick_panics": 0
}
```

## 6. Actions (UI → core)

`decision_act(id, "retry")` is new for `delivery`. Everything else goes through existing actions.
