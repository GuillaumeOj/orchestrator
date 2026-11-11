# Add Mention In Overwatch · API contract

> UI ↔ core (`chat_send`) and core ↔ Overwatch (`user_message` event). Field names are final.

## 1. `chat_send` (UI → core)

```jsonc
chat_send {
  "text": "what is @add-mention-in-overwatch waiting on?",
  "thread": null,                       // unchanged
  "context": {                          // optional; both keys optional, may be together
    "plan": "p_3",                      // unchanged (P4.5)
    "mentions": [ { "kind": "feature", "id": "f_42" } ]
  }
}
→ { "entry": ChatEntry }
```

- `mentions`: array of ≤ 10 items, each `{kind, id}`; `kind` is `"feature"` only (others reserved). Duplicate ids collapse.
- Errors (`bad_request: …`, the message stays unsent at the foot as today): `mentions` not an array, an item not `{kind: string, id: string}`, an unknown `kind`, more than 10 items.
- An `id` that is not a feature in the store (deleted meanwhile) is **dropped**, not an error; an archived feature is kept.
- The text is not parsed by the core: what is mentioned is what `context.mentions` says.

### Entry meta

The `user` / `text` entry gains, when at least one mention resolved:

```jsonc
"meta": {
  "delivery": "waking",
  "plan": { … },                        // unchanged
  "mentions": [
    { "kind": "feature", "id": "f_42", "slug": "add-mention-in-overwatch",
      "title": "Add Mention In Overwatch", "project_id": "p_1", "project": "orchestrator" }
  ]
}
```

The UI renders `@<slug>` tokens of the text that match a `meta.mentions[].slug` as links to `id` (spec §3.3). Pending messages rebuilt on restart read `meta.mentions[].id`.

## 2. `user_message` event (core → Overwatch)

`Event` gains `mentions: Option<Vec<Value>>` (serde default, skipped when none), resolved when the event is built (at send; again when a pending message is rebuilt from its entry):

```jsonc
"mentions": [
  {
    "kind": "feature",
    "id": "f_42",
    "slug": "add-mention-in-overwatch",
    "title": "Add Mention In Overwatch",
    "project": "orchestrator",           // name
    "project_path": "/Users/…/orchestrator",
    "run": "run_17",                     // null when the feature has no run
    "status": "executing",               // run status, null without a run
    "branch": "orch/add-mention-in-overwatch/integration",
    "docs_dir": "/…/integration/.orchestrator/features/add-mention-in-overwatch", // null if missing
    "worktree": "/…/integration",        // integration worktree, null if none
    "agents": ["add-mention-in-overwatch-lead-1", "…"], // live team agents, ≤ 12
    "pr_url": null
  }
]
```

`Event::line` appends ` · mentions <compact json>` after ` · plan …`, e.g.

```
- [ev-12] user_message · what is @add-mention-in-overwatch waiting on? · mentions [{"kind":"feature","id":"f_42",…}]
```

## 3. Audit

`chat_send` audit detail: `{"chars": n, "mentions": m}` (`m` = resolved count).
