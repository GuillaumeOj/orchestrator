# P4.3 — API contract (core ↔ UI)

> Companion of [spec.md](spec.md). Back owns the Rust side, front the Svelte side; front may mock §2 until back lands it. Field names are final; adding optional fields later is allowed, renaming is not. Everything in P4.1's contract ([p-4-1-one-now/api-contract.md](../p-4-1-one-now/api-contract.md)) stands: Decisions, context blocks, lifecycle, outcomes, `inbox_act`, the `now` event.

## 1. Chat entry

```jsonc
// ChatEntry
{
  "id": "c-812",                    // stable, unique for ever (a returning synthesized decision id gets a new entry)
  "day": "2026-11-02",              // the local working day it belongs to (digest::today)
  "at_ms": 1793600000000,           // posted
  "author": "core",                 // core | overwatch | user        (P4.3 writes core only)
  "kind": "decision",               // decision | morning            (P4.3)
                                    // | text | annotation | rule_proposal   (reserved, P4.4)
  "thread": "run-12",               // run id, else agent id; absent for morning
  "decision": "inbox-42",           // kind decision: the Decision id (matches now.decisions[].id while listed)
  "text": "Good morning. 1 run in progress, 1 decision waiting, week 38 % against 35 % planned.",
                                    // morning (and P4.4 text); absent for decision. Parts at zero left out; all zero:
                                    // "Good morning. Nothing is running and nothing is waiting for you."; no plan: no week part (ux F5)
  "line": {                         // kind decision, once it closed; absent while open / parked
    "verb": "allowed",              // allowed | denied | answered | approved | sent back | rejected | restarted
                                    // | applied | skipped | kept queued | run now | dismissed | expired | interrupted | failed
    "agent": "back-1",              // absent when the decision has no agent
    "what": "cargo add serde_json", // short input or title, ≤ 48 chars, core-cut
    "by": "rule",                   // you | rule | pane | expired | core | overwatch
    "by_detail": "installs",        // rule: the rule's label or id; overwatch: reason (P4.4)
    "state": "effect",              // the final lifecycle state
    "at_ms": 1793600060000          // decided (or closed) at
  },
  "card": { /* kind decision, once closed: the Decision as it was (P4.1 §2.1 shape, payload omitted) */ },
  "continued_from": "c-790",        // optional: open across midnight (§1, F1)
  "continued_to": "c-812",          // optional: on the previous day's entry
  "rev": 3                          // +1 on every write of this entry; keep the highest
}
```
- A decision entry is written when the decision is first listed open; `line` and `card` are added when it closes (one write). A permission answered by a rule, in the pane or by expiry without ever being listed open is posted already closed (`line` and `card` set in the first write).
- The UI renders a decision entry from `now.decisions[decision]` while it is listed, else from `card`. The line text is UI-composed from `line` (copy: ux), e.g. `▸ allowed back-1: cargo add serde_json · rule: installs`.
- `by` maps from `lifecycle.decided_by`: `user` → you, `rule:<id>` → rule (`by_detail` = label or id), `pane`, `expired`, `core`, `overwatch` as is.
- **Synthesized decisions that clear by themselves** (`error-`, `wait-`, `trust-`, `ask-` leaving `now` when their condition ends) close their entry with `line {verb: "dismissed", by: "core", state: "stale"}` (ux F8).
- **Open across midnight** (ux F1, data fix): at the day rollover, every decision still open gets a **new entry in the new day** (new id, same `decision`, `continued_from: "<old entry id>"`); the old entry keeps no `line` and gets `continued_to: "<new entry id>"`. An entry with no `line` in a past day is "left open" (UI copy). `chat_day` and `chat_search` return such entries as they are.
- **Expired** lines (ux F10) are for decisions with a default (ask, budget) and a parked permission the core dismissed; a permission hold that expires parks and stays an open card.

## 2. Push

### 2.1 `now` event (adds one field)
```jsonc
{ "rev": 1834, …, "chat": [ ChatEntry… ] }   // new and changed entries (upsert by id, keep highest `rev`); absent when none
// "topics" gains "chat" when any entry changed (a rule-answered line wakes the publisher at once)
```
A new working day starts a new thread: the first entry of a new `day` tells the UI to start today's thread over (entries of the previous day move to "older days").

### 2.2 `now_snapshot` (adds one field)
```jsonc
{ …, "chat": { "day": "2026-11-02", "entries": [ ChatEntry… ] } }   // today's entries, oldest first
```

## 3. Commands (Tauri; `orchd` server method of the same name)

| Command | Args | Returns |
|---|---|---|
| `chat_days` | `{ limit? = 30, before? : "YYYY-MM-DD" }` | `[ { "day": "2026-11-01", "entries": 41, "decisions": 12 } ]` newest first, days with at least one entry, today excluded |
| `chat_day` | `{ day: "YYYY-MM-DD" }` | `{ "day", "entries": [ ChatEntry… ], "outcomes": [ Outcome… ] }` — entries oldest first; outcomes of that day (P4.1 §2.4, rebuilt from events, `seen` omitted) |
| `chat_search` | `{ query, limit? = 50 }` | `[ { "day", "entry": ChatEntry } ]` newest first; matches the entry's `text`, `line.what`, `line.agent`, `line.verb`, `line.by_detail`, the card's `title` and `summary`, and for entries still open the live decision's `title` and `summary` (ux F9); case-insensitive substring; today included |

Errors are the usual `{ code, message }`; an unknown or malformed day is `bad_request`.

## 4. Notifications and `focus_card`

- A notification for one decision carries its id; a batched notification carries the oldest one of the batch. Titles and bodies: [p4-3-design-notes.md](../../../docs/design/p4-3-design-notes.md) §8.3 (ux F7).
- When the user clicks it (or, if the platform gives no click callback, when the app is activated within 60 s of the last notification; spec §3.5), the app emits the Tauri event:
```jsonc
// event "focus_card"
{ "id": "inbox-42" }
```
- The UI focuses that card in the chat (closing the Inspector overlay). If the decision is no longer open, it focuses the next open card and shows a toast with what happened to it (its `line`, e.g. "Allowed in the pane · 14:02").

## 5. `orchd`

`orchd chat [--day YYYY-MM-DD] [--search TEXT]`: prints a day's entries (default today), one per line: `14:02 core decision inbox-42 allowed back-1: cargo add serde_json · rule: installs`.
