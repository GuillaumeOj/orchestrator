# Fix Overwatch trust folder — spec

## 1. Problem

The Overwatch runs one Claude Code session per day, in its day folder
`<app data>/overwatch/<YYYY-MM-DD>/` (`Runtime::overwatch_day_dir`,
`overwatch/session.rs`). The folder is new every day, so Claude Code shows its
workspace trust dialog ("Quick safety check") at the day's first spawn. The app
detects it on the PTY and, per SPEC J0, never answers it: the Overwatch stays
stuck (no SessionStart, nothing delivered) until the user opens the Overwatch's
pane and picks "Yes, I trust this folder". Every day.

User's brief: always trust this kind of folder (preferred), or show the
warning in the Overwatch chat.

## 2. Decision

1. **Auto-trust (primary).** The runtime answers the trust dialog itself,
   **for the Overwatch's session only**, and only when its working folder is
   the app's own day folder. That folder is created by the app and only holds
   files the app writes (`briefing.md`, `events.seq`); trusting it is not a
   decision about the user's code. Every other agent keeps J0: the app never
   answers their dialog.
2. **Chat notice (fallback).** If the dialog is still on screen after the
   auto-answer window (Claude Code changed the dialog, the answer did not
   take), the Overwatch shows as **paused: waiting for folder trust** and the
   chat column shows a strip that opens its session, so the user answers it
   there. The Inbox trust card stays as today in that case.

Rejected:
- Writing `hasTrustDialogAccepted` into `~/.claude.json`: breaks FR-1206 (the
  app never writes the user's Claude Code config) and races Claude Code's own
  writes of that file.
- A stable working folder (`overwatch/` for every day, day folder as an added
  dir) or a git repo around the day folders so trust is asked once: still one
  manual answer, depends on how Claude Code keys trust, and moves the session's
  day out of `worktree`'s name (`session_day`) and its transcripts' folder.

## 3. Behaviour

### 3.1 Which session is auto-trusted

All of these hold, checked when the dialog is detected:
- the agent record `is_overwatch()` (id `overwatch`);
- its `worktree` is `cfg.paths.root/overwatch/<day>` (compared after
  canonicalising both; a path elsewhere is never auto-trusted);
- no answer was already tried for this spawn.

### 3.2 Answering

The dialog's options are `1. Yes, I trust this folder` and `2. No, exit`; the
selected one is marked `❯`. SPEC J0 observed the default on "No, exit", but the
runtime must not rely on which one is selected:
- read the latest render of the dialog from the PTY tail;
- if `❯` is on "Yes, I trust this folder", send Enter;
- otherwise send one arrow key towards "Yes" (Up), wait for the re-render
  (≤ 1 s), and check again; Enter only once "Yes" is selected;
- **never** send Enter while "No, exit" (or anything that is not "Yes, I trust
  this folder") is selected, and never send Esc (it exits);
- at most 3 arrow keys per spawn; then give up (fallback).

Keys go to the PTY like the runtime's other programmatic input, but this is
not a turn: no Governor admission, no marker, no paste.

Event `overwatch.trust_answered {dir}` when Enter was sent; event
`overwatch.trust_failed {dir, why}` on give-up.

### 3.3 While auto-answering

The auto-answer window is **10 s** from the dialog's detection.
- Delivery stays blocked while the dialog is on screen (nothing is pasted into
  it), as today.
- The Overwatch's trust dialog does **not** raise the Inbox trust card, the
  `trust-overwatch` decision, nor a notification during the window. The user
  sees nothing; the Overwatch status stays `waking` / `working` as it would.
- `watch_startups` (FR-415) keeps skipping an agent with the dialog on screen.
- The first SessionStart clears the detection as today; the session then gets
  its briefing.

### 3.4 Fallback

When the window ends with the dialog still on screen, or the answer gave up:
- the record's `trust_dialog` is raised as today: the Inbox trust card and the
  `trust-overwatch` decision appear (their copy for the Overwatch: "Claude Code
  asks to trust today's Overwatch folder" / "The app could not answer it: answer
  it in its session");
- `now.overwatch` becomes `status: "paused"`, `why: "waiting for folder trust"`
  (`why::TRUST`), `has_session: true`;
- the chat column shows the strip (see §4);
- once the user answers (SessionStart), all of it clears and the Overwatch goes
  on (`waking` / `working`).

An Overwatch that is `off` never spawns, so none of this applies.

## 4. UI (chat column, fallback only)

No designer on this feature; the strip reuses the existing "down" strip under
the chat header (`stripText`, `ui/src/lib/overwatch.js`), same component and
tokens.

- Strip, while `ow.status === 'paused' && ow.why === 'waiting for folder trust'`:
  - title: **Claude Code asks to trust today's Overwatch folder**
  - detail: **Open its session and pick "Yes, I trust this folder" · cards still work**
  - an action **Open session** (same as clicking the header status: opens the
    Overwatch's session pane; ⏎ when focused).
- Header status: `paused` (existing). Tooltip: "Overwatch is paused: waiting
  for folder trust" (existing paused copy).
- Input hint: existing paused hint ("Paused · it answers when it resumes").
- Other paused reasons (Pause all, limit reached) keep no strip.

## 5. Docs to update

- `docs/SPEC.md` J0 step 2 and the ADR-002 / ADR-003 trust lines: "never
  answers it itself" gets the Overwatch exception (§2.1).
- `crates/agent-runtime/README.md` (Overwatch section): auto-trust and fallback.
- `decision.rs` copy "Answer it in {}'s pane: the app never does" stays for
  other agents; the Overwatch gets the §3.4 copy.

## 6. Tests

Backend (Rust):
- unit: the option parser on captured dialog renders (selection on Yes, on No,
  a re-render after an arrow, an unknown layout → no key sent);
- unit: §3.1 guard (overwatch in its day folder → yes; overwatch record with a
  worktree elsewhere, any other agent → never);
- e2e with `fake_claude.sh` given a fake trust dialog (env switch: prints the
  dialog with "No, exit" selected, moves on Up, starts on Enter, exits on a
  wrong Enter): the Overwatch starts with no Inbox card and no
  `trust_dialog` record flag; a variant whose dialog never takes the answer
  ends in the fallback (`paused`, `why::TRUST`, Inbox card) and clears on
  SessionStart;
- a regular agent with the fake dialog still gets the Inbox card and no key.

Frontend (vitest): `stripText` / the strip for the trust reason, its Open
session action, no strip for the other paused reasons.

## 7. Out of scope

- Trust for project repos and worktrees (J0 unchanged).
- Changing where the Overwatch's day folder lives.
