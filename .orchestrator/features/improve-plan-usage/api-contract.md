# Improve Plan Usage · API contract

## Orchestrator MCP tools (lead)

`create_task` gains:

```json
"model": {"type": "string", "description": "Run this task on this model (alias `opus`, `sonnet`, `haiku`, `fable`, or a model id) instead of the node's. For a hard task, or one that failed a review round."}
```

`update_task` gains the same `model` (lead only; `""` clears it). It takes effect at the next task boundary of the owner; a task already running keeps its model.

Validation: trimmed; empty → none; refused if it contains whitespace or is longer than 64 chars.

## Store / UI payloads

`Task.flow.model?: string` (omitted when none, `skip_serializing_if`). The UI reads it from the task payloads it already gets (no new endpoint).

`RoleDef.model` and `RoleOverrides.model` are unchanged (`string | null`); the UI's select only constrains what it writes:

| Select | Saved value |
|---|---|
| default (Claude Code's) | `null` |
| opus / sonnet / haiku / fable | the alias |
| custom… | the trimmed text (`null` when empty) |

## Runtime event

`model_switch` keeps `{from, to}` and adds `why`: `"task"` (task model), `"downgrade"` (FR-819), `"restore"` (back to the role's model).
