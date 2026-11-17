# Allow Long Text In Overwatch · Feature spec

> Feature `allow-long-text-in-overwatch` · Integration branch `orch/allow-long-text-in-overwatch/integration`
> Companions: [api-contract.md](api-contract.md) (UI ↔ core, core ↔ Overwatch) · [ux-brief.md](ux-brief.md) (interaction, states, copy, boards to draw)
> Builds on: P4.4 chat with the Overwatch ([p-4-4-the-overwatch](../p-4-4-the-overwatch/spec.md) §3.9), mentions on `chat_send` ([add-mention-in-overwatch](../add-mention-in-overwatch/spec.md)).

## 1. Goal

Brief: *"Allow to paste long text in the Overwatch chat field. It could then be replaced by a placeholder like `[pasted text #1 xyz chars]` (basically like in the Claude Code CLI). This means automatically pasting the text in a temp .md file."*

The user pastes a long text (a log, a stack trace, a spec) into the Overwatch input. The field shows a short placeholder `[Pasted text #1 · 3 214 chars]` instead of the text, the user keeps typing around it and sends. The core writes each pasted text to a `.md` file the Overwatch can read, the Overwatch's turn names the file for each placeholder, and the sent message shows the placeholder as a chip that opens the pasted text.

Note: the brief says it "mentions GitHub issue #1". Issue #1 of the repository is the closed CI / Renovate / Biome PR, unrelated to this feature; the `#1` in the brief is the placeholder example. No issue is linked to this feature.

**Exit criterion**: in the app, pasting 5 000 characters into the Overwatch input inserts `[Pasted text #1 · 5 000 chars]` at the caret; sending it writes the text to `<app root>/overwatch/pastes/<day>/…-1.md`, delivers a `user_message` whose line lists that file, the Overwatch `Read`s it and answers about its content; the user's entry shows the placeholder as a chip that expands to the pasted text.

## 2. What the code does today

| Item | Today (main 540838fe) |
|---|---|
| Input | `ui/src/components/chat/ChatInput.svelte`: a `textarea` bound to `app.chatDraft` (kept while the app is open, never saved). A paste is inserted as is. A counter shows from 1 800 code points; over `MAX_CHARS` = 2 000 (`ui/src/lib/overwatch.js`) the send is blocked with "Too long for a message. Put it in a file and ask the Overwatch to read it." |
| Send | `ChatColumn.svelte` `sendMessage` → `invoke('chat_send', {text, context?})`; `context` is `{plan?, mentions?}`. A refused send stays at the foot with Retry (`unsent`). |
| Core | `Runtime::chat_send_with` (`agent-runtime/src/overwatch/chatw.rs`): trims, refuses empty or > `SEND_MAX` (2 000 chars), writes a `user` / `text` entry (`meta.delivery`, `meta.plan`, `meta.mentions`), audits, enqueues a `user_message` event (`overwatch/session.rs` `overwatch_user_message_with`). Pending messages are rebuilt from the entry's meta on restart (`overwatch_load_db_pending`). `Event::line` (`overwatch/mod.rs`) appends ` · plan <json>`, ` · mentions <json>`. |
| Overwatch reads | `overwatch_reads` (`session.rs`): project paths, worktrees, the day dir `<root>/overwatch/<day>`, its `messages/`. A long turn body already goes to a `messages/<turn>.md` file it is told to `Read` (`delivery::prepare`, `MAX_PASTE_CHARS` = 900). |
| User entries | `ChatText.svelte` renders the user's text as plain text, with mention links. |

## 3. Scope

### 3.1 Design (S) — ux, then ui
ux finalises the interaction in `design/design-notes.md` from [ux-brief.md](ux-brief.md) (states, keys, copy, edge cases), hands off to ui; ui draws the boards of ux-brief §5 in the Drive `Orchestrator.figcraft` (P4.4 · 04.3 chat column page, next to the mention boards) and exports PNGs to `design/`. Copy and visuals in the notes are final for front; behaviour changes go through lead.

### 3.2 Paste in the input (M) — front
- **When a paste becomes a placeholder**: a plain-text paste (`clipboardData.getData('text/plain')`) of **more than 800 characters or more than 10 lines**. A shorter paste is inserted as today. Non-text pastes (images, files) are unchanged (out of scope).
- **Placeholder**: `[Pasted text #<n> · <chars> chars]`, `<chars>` the code-point count of the pasted text formatted like the counter (`3 214`, thin grouping as `spaced()` does today), `<n>` = 1, 2, … in the order of pastes in this draft (never reused within a draft, restarting at 1 after a send or when the draft is emptied). It replaces the selection at the caret, with no extra spaces added. The caret lands after it. ⌘Z undoes the insert (insert through the textarea's own editing, e.g. `execCommand('insertText')`, not by setting the value).
- **Kept next to the draft**: `app.chatPastes = [{n, text, chars, lines}]` beside `app.chatMentions`, cleared with the draft on send. Like the draft it lives while the app is open and is never saved.
- **Editing**: Backspace with the caret just after a placeholder (or Delete just before) removes the whole placeholder. Any placeholder still present, whole and unchanged, at send is sent; a paste whose placeholder was removed or edited is dropped. Typing inside a placeholder is allowed (it then stops being one). Copying a placeholder out of the field copies the placeholder text, not the paste.
- **Limits**: one paste at most **200 000 characters**; at most **10 pastes** per message; the typed text including placeholders stays ≤ 2 000 (the counter and the over-length block are unchanged and count placeholders as their text). A paste over 200 000 characters is not inserted; a short inline error says so (ux copy). An 11th paste: same, "At most 10 pasted texts in one message".
- **Over-length message**: the copy changes to tell the user that pasting works (ux copy, e.g. "Too long for a message. Paste long text: it is sent as a file.").
- **At send**: `context.pastes = [{n, text}]` for the pastes still referenced, beside `plan` and `mentions`. A message made only of placeholders is valid. A retry of an unsent message keeps its pastes.
- **Planning tag / mentions**: unchanged and combinable; `@` typed inside a placeholder does not open the mention popup.

### 3.3 Pastes in the thread (S) — front
- In a **user** text entry, each placeholder token whose `n` is in `meta.pastes` renders as a **paste chip** (ux-brief §3) showing `Pasted text #n · 3 214 chars · 80 lines`. Click / ⏎ / Space toggles an inline expansion under the bubble showing the pasted text (mono, pre-wrapped, max height ~ 16 lines then scroll, a Copy button). The text is fetched with `chat_paste` (api-contract §1.2) on first open and kept for the session. A file since gone: the chip is muted and the expansion says "Pasted text no longer available".
- Unsent / waking entries render their chips the same, from the pastes in memory.
- Search (`/`) matches the entry's raw text (the placeholders), not the pasted content.

### 3.4 Core (S) — back
- `chat_send` accepts `context.pastes` (api-contract §1.1). Validation: an array of ≤ 10 `{n: 1..=99, text: non-empty string ≤ 200 000 chars}`, distinct `n`; a bad shape is `bad_request`. A paste whose placeholder prefix `[Pasted text #<n> ` is not in the trimmed text is dropped (logged). `SEND_MAX` is unchanged and applies to the text only.
- Each kept paste is written, **before** the entry is inserted, to `<root>/overwatch/pastes/<day>/<key>-<n>.md` (`<key>` a fresh short random id of the send, `<day>` the chat day), UTF-8, the text as pasted (no header). If writing or the insert fails, the files written are removed and the send fails.
- The entry's `meta.pastes = [{n, chars, lines, path}]`. The `user_message` event carries the same list (`Event.pastes`), rebuilt from the meta for pending messages on restart; `Event::line` appends ` · pastes <json>` as it does for `mentions`.
- `overwatch_reads` adds `<root>/overwatch/pastes` (all days: a pending message of an earlier day must stay readable).
- New command `chat_paste(entry, n)` → `{n, text, chars, lines}` reading the file named in the entry's meta; `not_found` when the entry, the paste or the file is missing. Only paths under `<root>/overwatch/pastes` are read.
- `overwatch/role.md`: one paragraph: in a `user_message` with `pastes`, each `[Pasted text #n …]` placeholder stands for the file listed for `n`; `Read` it before answering when the message depends on it; refer to it as "pasted text #n".
- Audit: `chat_send` detail gains `pastes: <count>`, `paste_chars: <total>`.
- Retention: paste files live as long as the chat history (no cleanup in this feature, like the day dirs).

## 4. Out of scope
- Pasting images or files; drag-and-drop of text (only the paste event).
- Pastes in other inputs (agent shells, the planner, settings).
- Editing a pasted text after it was pasted (remove the placeholder and paste again).
- Searching inside pasted content.
- Cleanup / retention of paste files.
- Raising `SEND_MAX` for typed text.

## 5. Non-functional
- A 200 000-character paste inserts its placeholder within one frame; the textarea never holds the long text.
- No IPC while typing or pasting; the one round-trip is the send.
- Tests: `make ci` green. Core: `chat_send` with pastes (files written under the day, meta, event line, unreferenced paste dropped, bad shape / too many / too long refused, with `plan` and `mentions` together, insert failure removes the files, pending rebuild keeps the pastes), `overwatch_reads` covers the pastes dir, `chat_paste` (ok, unknown entry / n, missing file). UI: threshold (800 chars / 10 lines, both sides), placeholder format and numbering, caret and selection replace, Backspace / Delete remove whole, edited placeholder dropped, limits and their errors, context sent with plan and mentions, retry keeps pastes, chip rendering, expand / fetch / missing file, `@` inside a placeholder. Tests hold at any date, hour and time zone.

## 6. Tasks (planned)

| # | Node | Task | Depends on |
|---|---|---|---|
| 1 | ux | Design notes: interaction, states, copy (ux-brief) | — |
| 2 | ui | Boards in the Drive FigCraft file, PNG exports to `design/` | 1 |
| 3 | back | Core: `chat_send` pastes, files, event, reads, `chat_paste`, role.md | — |
| 4 | front | Input placeholder + thread chips (§3.2–3.3) | 2, 3 |
