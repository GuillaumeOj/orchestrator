# Add Mention In Overwatch · Feature spec

> Feature `add-mention-in-overwatch` · Integration branch `orch/add-mention-in-overwatch/integration`
> Companions: [api-contract.md](api-contract.md) (UI ↔ core, core ↔ Overwatch) · [ux-brief.md](ux-brief.md) (interaction, states, copy, boards to draw)
> Builds on: P4.4 chat with the Overwatch ([p-4-4-the-overwatch](../p-4-4-the-overwatch/spec.md) §3.9), P4.5 planning context on `chat_send` ([p-4-5-intake](../p-4-5-intake/api-contract.md) §3.3).

## 1. Goal

Brief: *"Add a @mention feature in the Overwatch chat. The most important is to be able to mention a current feature. We need an autocompletion helper for this."*

The user types `@` in the Overwatch input, picks a feature from an autocomplete list, and sends. The Overwatch receives, with the message, which feature(s) were meant and where to look (run, status, agents, docs, worktree), so it can answer about that feature without guessing. The sent message shows the mention as a link to the feature.

**Exit criterion**: in the app, typing `@add` in the input lists the matching features, ⏎ inserts `@add-mention-in-overwatch `, sending delivers a `user_message` event whose line carries the feature's run, status, agents and docs path; the user's entry shows `@add-mention-in-overwatch` as a link that opens the feature pane.

## 2. What the code does today

| Item | Today (main 95d930f1) |
|---|---|
| Input | `ui/src/components/chat/ChatInput.svelte`: a `textarea` bound to `app.chatDraft`; ⏎ sends, ⇧⏎ newline, Esc leaves to the thread. No completion of any kind. |
| Send | `ChatColumn.svelte` `sendMessage` → `invoke('chat_send', {text, context?})`; `context` is only `{plan: <project id>}` (P4.5). A refused send stays at the foot with Retry (`unsent`). |
| Core | `Runtime::chat_send_with` (`agent-runtime/src/overwatch/chatw.rs`): validates, writes a `user` / `text` chat entry (`meta.delivery`, `meta.plan`), enqueues a `user_message` event (`overwatch/session.rs` `overwatch_user_message_with`); pending messages are rebuilt from the entry's meta on restart (`session.rs` ~l.726). `Event::line` (`overwatch/mod.rs`) appends ` · plan <json>`. |
| Overwatch | `inspect` takes an agent id only; `world` lists runs. A feature named in plain words must be guessed from `world()`. Role prompt `overwatch/role.md`. |
| Features in the UI | `app.data.features` (`features()` in `lib/app.svelte.js`): `{id, project_id, slug, title, branch, archived, run: {id, status, …} | null, tasks}`; `openFeature(id)`; fuzzy matcher `lib/fuzzy.js` (used by ⌘P, which already lists features as `<project> › <slug>`). |
| User entries | `ChatText.svelte` renders the user's text as plain text (the Overwatch's as markdown). |

## 3. Scope

### 3.1 Design (S) — ux, then ui
ux finalises the interaction in `design/design-notes.md` from [ux-brief.md](ux-brief.md) (states, keys, copy, edge cases), hands off to ui; ui draws the boards of ux-brief §6 in the Drive `Orchestrator.figcraft` (P4.4 04.3 chat column page) and exports PNGs to `design/`. Copy and states in the notes are final for front.

### 3.2 Autocomplete in the input (M) — front
- **Trigger**: `@` typed at the start of the field or after whitespace or `(` `[` `"` `'`. Not inside a word (`me@x.com` does nothing) and not inside an inline code span (odd number of backticks before it on the line). The query is the text between the `@` and the caret, chars `[A-Za-z0-9._/-]`; the popup closes on a space, on any other char, when the caret leaves the token, on blur, or on Esc.
- **Candidates**: the non-archived features of `app.data.features`. **Current** features first (a run whose status is not `done` / `failed`), then the others. Within each group, by fuzzy score (`fuzzy.score`) of the query against `slug`, then `title`, then `<project name>/<slug>` (best of the three); empty query: by most recent run activity / creation. At most 8 rows visible, the list scrolls.
- **Row** (ux-brief §3): slug (mono), title (muted, truncated), project name, run state label (`runStateLabel`) or "No run". Current group / other group separated as ux draws it.
- **Keys** while the popup is open: ↑ / ↓ (and ⌃P / ⌃N) move, ⏎ or ⇥ picks, Esc closes the popup only (a second Esc leaves the input as today), ⏎ never sends while the popup is open. Mouse: hover highlights, click picks. Opening the popup highlights the first row.
- **Pick**: replaces `@<query>` with `@<slug> ` (one trailing space; no extra space if one already follows) and keeps the caret after it. The pick is remembered next to the draft: `app.chatMentions = [{token: "@<slug>", kind: "feature", id}]`, cleared with the draft on send.
- **At send**: the mentions sent are the remembered picks whose token is still in the text as a whole token, plus any `@<slug>` typed by hand that matches **exactly one** non-archived feature. Duplicates collapse; at most 10. `context.mentions = [{kind: "feature", id}]` is added beside `context.plan` (both may be present). A retry of an unsent message keeps its mentions.
- **Accessibility**: combobox pattern on the textarea (`aria-autocomplete="list"`, `aria-expanded`, `aria-controls`, `aria-activedescendant`), the popup a `listbox` of `option`s; the result count announced in the existing live region ("3 features", "No feature matches").

### 3.3 Mentions in the thread (S) — front
- In a **user** text entry, each `@<slug>` token that matches an entry of `meta.mentions` renders as a mention link (ux-brief §4): click / ⏎ opens the feature pane (`openFeature(id)`). A feature since removed: plain token, muted, tooltip "Feature removed". Tokens with no mention stay plain text. Unsent / waking entries render their mentions the same (from the context they were sent with).
- Search (`/`) matches the raw text (unchanged).

### 3.4 Core (S) — back
- `chat_send` accepts `context.mentions` (api-contract §1). Each id is resolved against the store; unknown ids are dropped (logged), a bad shape is `bad_request`. The entry's `meta.mentions` keeps the resolved snapshot `{kind, id, slug, title, project_id, project}`.
- The `user_message` event carries `mentions` resolved **when the event is built** (at send, and again when a pending message is rebuilt from its meta on restart): feature, slug, title, project, run id and status, integration branch, feature docs dir, integration worktree, the team's agent ids, PR url (api-contract §2). `Event::line` appends ` · mentions <json>` as it does for `plan`. Size: a turn's event text stays bounded (≤ 10 mentions, agents list ≤ 12 ids).
- `overwatch/role.md`: one paragraph: a `user_message` with `mentions` is about those features; use the ids to `inspect` their agents and `Read` their docs, and name the feature as `@<slug>` in the reply.
- Audit: `chat_send` audit detail gains `mentions: <n>`.

## 4. Out of scope
- Mentioning agents, projects, decisions or files (the contract keeps a `kind` field so they can come later).
- Rendering `@slug` in the Overwatch's own replies as links (it is told to write `@slug`; linking it is a follow-up).
- Highlighting mentions inside the textarea while typing (a plain textarea; no rich editor).
- A new Overwatch tool (e.g. `inspect` by feature); the event payload is enough.
- Notifications on mention.

## 5. Non-functional
- No new IPC round-trip while typing: candidates come from `app.data.features` already in memory.
- Popup opens within one frame of the keystroke; filtering 200 features stays under 2 ms.
- Tests: `make ci` green. Core: `chat_send` with mentions (resolved meta, event line, unknown id dropped, bad shape / too many refused, with `plan` together, pending rebuild re-resolves). UI: trigger rules (start, after space, not in a word, not in code), ranking (current first, fuzzy), keys (↑↓ ⏎ ⇥ Esc, ⏎ doesn't send while open), insertion and trailing space, mentions sent (picked kept / deleted dropped / typed exact unique match / ambiguous typed not sent), context with plan, chip rendering and removed feature, a11y attributes. Tests hold at any date, hour and time zone.

## 6. Acceptance (demo at the PR gate)
1. With two projects running features, `@` lists current features first with their project and state; `@ment` narrows to this feature; ⏎ inserts `@add-mention-in-overwatch `.
2. Esc closes the list and keeps the text; a second Esc leaves the input; `me@x.com` opens nothing.
3. Send "what is @add-mention-in-overwatch waiting on?": the Overwatch's turn shows ` · mentions [...]` with run, status, agents, docs dir; it answers about that feature, naming it `@add-mention-in-overwatch`.
4. The sent entry shows the mention as a link; clicking it opens the feature pane.
5. Boards of ux-brief §6 exported under `design/`.

## 7. Tasks (after spec approval)
1. **ux** — design-notes.md (docs only). Merged first; then the integration branch is rebased on main (it is 174 files behind, `ChatInput.svelte` included) before the code tasks are created.
2. **ui** — boards and exports (depends on 1).
3. **back** — §3.4 (after the rebase; independent of design).
4. **front** — §3.2, §3.3 (depends on 2; against api-contract, syncs back's branch for the end-to-end check).

## 8. Decisions after the ux pass (lead, 2026-11-11)
[design/design-notes.md](design/design-notes.md) is final for copy and visuals. On its "Changes to the spec":
- **1–6 accepted** as written: ⏎ / ⇥ with no match or no features close the popup, keep the text, send nothing, and the popup reopens on the next edit of the token (also after Esc); the popup opens with "No features yet"; group headers only when both groups have rows; popup = the input block's full width, above the planning tag; footer hint and title dropped under 340 px.
- **7, same slug in two projects**: when the picked feature's slug is shared by another non-archived feature, the inserted token is **`@<project key>/<slug>`**, where project key = the project name lowercased, each run of characters outside `[a-z0-9._-]` replaced by `-`, trimmed of `-`. Otherwise `@<slug>`. A mention matches both forms when rendering (longest first); a hand-typed `@<project key>/<slug>` resolves exactly like a hand-typed unique `@<slug>`. No contract change (the meta snapshot has `slug` and `project`).
- **8, token boundary**: a token ends at the first character outside `[A-Za-z0-9._/-]`, then trailing `.` `/` `-` `_` are trimmed (`@add-mention-in-overwatch.` → `@add-mention-in-overwatch`). Matching (rendering, "still in the text", hand-typed) uses the whole trimmed token, so `@add` never matches inside `@add-mention-in-overwatch`.
- **"Current"** stays any run not `done` / `failed` (draft, paused, deferred included: the user may well ask why they are stuck).
- **api-contract**: the event's mention uses `id` (not `feature`), like the meta and the context.
