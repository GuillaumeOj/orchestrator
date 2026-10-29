# Improve Agent Usage · API contract

## Run event `agent.fresh_session`

Recorded with `run_event(run_id, Some(agent_id), "agent.fresh_session", detail)` when a team agent's session is rotated to start a new task. It reaches the UI like every other run event (Feature pane timeline).

```json
{ "node": "back", "task": 3, "title": "Add the usage endpoint" }
```

| Field | Type | Meaning |
|---|---|---|
| `node` | string | The node the agent sits on. |
| `task` | number | The task id the fresh session starts (shown as `T-<task>`). |
| `title` | string | The task title. |

The event's `agent` column is the agent id (e.g. `improve-agent-usage-back-1`), so the timeline's agent chip shows who it is.

UI rendering: `T-<task> starts in a fresh session`, family **Run**.

## Agent event `fresh_session`

Agent-level event (`event(agent_id, "fresh_session", detail)`), for diagnostics only; no UI change.

```json
{ "old_session": "uuid", "new_session": "uuid", "task": 3 }
```

Distinct from `restarted_fresh` (startup-failure recovery, counted in the report).
