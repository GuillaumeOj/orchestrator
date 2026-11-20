# Allow Long Text In Overwatch · API contract

> UI ↔ core (Tauri commands) and core ↔ Overwatch (the `user_message` event). Behaviour: [spec.md](spec.md) §3.

## 0. The placeholder

The UI writes, in the message text, one token per paste:

```
[Pasted text #<n> · <chars> chars]
```

- `<n>`: integer 1..=99, unique within the message.
- `<chars>`: the paste's code-point count, display-formatted by the UI (`3 214`; the core never parses it).
- The core matches a paste to the text by the prefix `[Pasted text #<n> ` (with the trailing space, so `#1` does not match `#12`).
- The same token may appear more than once: every occurrence stands for the one paste `n` (one file).
- Counting, the same on both sides so the placeholder, the chip and `meta` agree: the UI normalises the pasted text (CRLF and lone CR → LF) before counting and sending; `chars` = code points; `lines` = Rust `str::lines().count()` (`"a"` → 1, `"a\n"` → 1, `"a\nb"` → 2). The core counts the text it receives and does not normalise again.

## 1. UI → core

### 1.1 `chat_send` (extended)

```jsonc
invoke('chat_send', {
  text: "Why does this fail?\n[Pasted text #1 · 5 000 chars]\nand [Pasted text #2 · 1 200 chars]",
  context: {
    plan: "proj-…",                       // optional, unchanged (P4.5)
    mentions: [{ kind: "feature", id }],  // optional, unchanged
    pastes: [                             // optional, new
      { n: 1, text: "<the pasted text, 1..=200 000 chars>" },
      { n: 2, text: "…" }
    ]
  }
})
// → { entry }   (unchanged shape; entry.meta.pastes is new, see §2)
```

Errors (`bad_request: …`, the existing error string form):
- `context.pastes is an array` · `a paste is {n, text}` · `paste n is 1..=99` · `paste n <n> is twice` · `at most 10 pastes` · `paste #<n> is empty` · `paste #<n> is at most 200000 characters`.
- Unchanged: empty text, text over `SEND_MAX` (2 000; placeholders count as their text).

A paste whose prefix is not in the trimmed text is dropped (not an error; `eprintln!` warn).

### 1.2 `chat_paste` (new)

```jsonc
invoke('chat_paste', { entry: "c-412", n: 1 })
// → { n: 1, text: "…", chars: 5000, lines: 120 }
```

Errors: `not_found: …` when the entry is unknown, has no paste `n`, or its file is gone. The core canonicalises the meta path (`fs::canonicalize`) and reads it only if it is under the canonicalised `<root>/overwatch/pastes`; anything else is `not_found`.

## 2. The chat entry

`entry.meta.pastes` (present only when the message kept at least one paste):

```jsonc
"pastes": [
  { "n": 1, "chars": 5000, "lines": 120,
    "path": "/…/Orchestrator/overwatch/pastes/2026-11-17/k3f9a2c1-1.md" }
]
```

- `chars`, `lines`: as §0.
- File: `<root>/overwatch/pastes/<day>/<key>-<n>.md`, `<key>` a fresh short random id per send, the text written as pasted.

## 3. Core → Overwatch: the `user_message` event

`Event` gains `pastes: Option<Vec<Value>>` (the meta list above). `Event::line` appends, after `mentions`:

```
- [ev-31] user_message · Why does this fail? [Pasted text #1 · 5 000 chars] … · pastes [{"n":1,"chars":5000,"lines":120,"path":"/…/k3f9a2c1-1.md"}]
```

Pending messages rebuilt on restart (`overwatch_load_db_pending`) take `pastes` from the entry's meta.

`overwatch_reads` includes `<root>/overwatch/pastes`.

`role.md` paragraph (wording back's to settle):

> A `user_message` may list `pastes`: each `[Pasted text #n · … chars]` in its text stands for the file given for `n`. `Read` that file before answering when the message depends on it, and call it "pasted text #n" in your reply.

## 4. UI state

`app.chatPastes = [{ n, text, chars, lines }]` next to `app.chatDraft` / `app.chatMentions`; in memory only, cleared on send. An unsent entry keeps its pastes with its context for Retry and for rendering its chips.
