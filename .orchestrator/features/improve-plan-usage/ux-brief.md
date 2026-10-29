# Improve Plan Usage · UX brief

No new screen and no design pass: two small controls that reuse existing patterns. UI reviews the frontend change.

## Role / node inspector: model select

- Where: Team canvas inspector, the `model` row (today a free-text input with placeholder "default"), next to the `effort` select. Same `.field` select style as effort.
- Options: `default (Claude Code's)`, `opus`, `sonnet`, `haiku`, `fable`, `custom…`.
- `custom…` reveals a text input under the select (mono, placeholder `claude-…`), saved on change.
- A stored value that is not a known alias shows `custom…` selected with its text in the input.
- On a node, the select shows the role's value as its placeholder option (e.g. `role: sonnet`) so the override reads as an override. Same behaviour as the effort override today.
- The node card's meta line (`model · effort · max n`) is unchanged.

## Task card: model chip

- When `flow.model` is set, a small mono chip with the alias (`opus`) next to the plan-first marker, with a tooltip "Runs on opus (set by the lead)".
- Nothing when unset: the default is the node's model, already shown on the node.
