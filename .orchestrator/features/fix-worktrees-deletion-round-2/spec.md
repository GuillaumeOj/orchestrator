# Fix Worktrees Deletion, Round 2 · Feature spec

> Feature `fix-worktrees-deletion-round-2` · Integration branch `orch/fix-worktrees-deletion-round-2/integration` · Base `main` @ 95d930f (PR #60 `fix-remove-worktree` included) · back + front, no design pass (no new UI)

## 1. Problem

The project's overview (`ui/src/panes/ProjectPane.svelte`) has a worktree list. The user clicks a worktree's delete icon and approves in the confirm dialog. **Nothing happens.** No toast appears, the worktree stays listed and is still on disk.

PR #60 (`fix-remove-worktree`) fixed the pane's reloads and the core's removal. It did not fix this.

**Goal:** approving the dialog removes the worktree. The row leaves the list with a `Removed …` toast, or a refusal shows its error toast.

## 2. Root cause (reproduced on `origin/main` @ 95d930f)

The confirm dialog never calls its callback.

`ui/src/components/Dialogs.svelte`, the generic `confirm` branch (the last `{:else}`, lines 483–484):

```svelte
const o = $derived(app.overlay);
…
onkeydown={(e) => onKey(e, () => (close(), o.onYes()))}
<form onsubmit={(e) => (e.preventDefault(), close(), o.onYes())}>
```

`close()` sets `app.overlay = null`. `o` is derived from `app.overlay`, so reading `o` afterwards gives `null`, and `o.onYes()` throws `TypeError: Cannot read properties of null (reading 'onYes')`. The dialog has already closed, so the user sees nothing.

**Proof:** a throwaway vitest (not committed) rendered `Dialogs` after `confirm('Remove worktree x?', …, onYes, 'Remove')` and submitted the form. `onYes` was called 0 times and the window got exactly that `TypeError`.

**Why the tests and PR #60 missed it:** every test that covers a confirm calls `app.overlay.onYes()` directly (`projects.test.js`, `ecosystem.test.js`, `windows.test.js`, `chat.test.js`, `decisions.test.js`, `mission-pane.test.js`). None of them submits the real dialog. The pattern has been there since the app shell (30cfef6). It may have started to fail with a recent Svelte change, for example the 5.57.2 → 5.57.3 bump of 2026-11-09 (c454a30). That is not verified and doesn't change the fix.

**Blast radius:** every `confirm()` in the app goes through this dialog, about 20 call sites: worktree removal and project removal (ProjectPane), kill and restart fresh, run now and cancel queue, the discard-changes guard of the settings windows, and confirms in DiffPane, FeaturePane, MissionPane, ChatColumn, PlannerWindow, McpSettings and IntegrationsSettings. They are all broken the same way, and the one fix repairs them all.

The core path (`worktree_remove` → `Runtime::remove_worktree`) is not involved. PR #60's runtime tests cover it, and the pane's reload logic (`fetchDetail` with `seq`) is correct once the callback runs.

## 3. Scope

### UI (`front`)

- **U1:** in `Dialogs.svelte`'s confirm branch, read the callback **before** closing. One function, used by both the submit and the ⏎ key, for example `function accept() { const yes = o?.onYes; close(); yes?.(); }`. Use the same "read first" rule as the other branches (`const ov = o`, lines 80 and 134).
- **U2:** check the rest of `Dialogs.svelte`, and the other overlay or window components, for the same pattern: a handler that clears `app.overlay` / `app.window` and then reads a value derived from it. Fix each one found. The gate task's audit (§5, T-0) lists the candidates.
- **U3, tests in `ui/test/` (vitest), through the real dialog, never `app.overlay.onYes()`:**
  - `Dialogs` with a confirm overlay: clicking the confirm button, ⏎ on the focused button (the native form submit) and ⌘⏎ (the `onKey` keydown path) each run `onYes` exactly once and leave `app.overlay` null. Cancel and Esc don't run it. A bare `keyDown` Enter does nothing (see `audit.md` §1), so the ⏎ test presses it on the button.
  - End to end in the project pane: render `ProjectPane` and `Dialogs` together, click a worktree's **Remove worktree…** button, then the dialog's **Remove** button. `worktree_remove` is invoked with `{ project, path, deleteBranch: true, discard: false }`, the row leaves the list and the `Removed …` toast shows. Do the same for the unmerged case (**Discard and remove** → `discard: true`).
  - One other caller through the dialog, for example `confirmKill` → `agent_action` kill, so the fix is seen to cover more than the pane.

### Core (`back`)

No code change. `back` writes the gate audit (T-0) and reviews T-1.

Out of scope: the core's worktree removal (done in PR #60), restyling the dialog, converting the existing tests that call `app.overlay.onYes()` (they stay as unit tests of the callers).

## 4. Acceptance

1. In the project pane, approving a worktree's removal removes it: the row leaves the list and a `Removed …` toast shows. A refusal (for example a live agent holding the worktree) shows the core's error toast and keeps the row.
2. The unmerged case asks **Discard and remove** and removes with `discard: true`.
3. Every other confirm in the app runs its action once it is approved, with the button, ⏎ or ⌘⏎, and does nothing on Cancel or Esc.
4. No `TypeError` in the console when a confirm is approved.
5. `bun run test`, `bun run check` and `bun run lint` pass, and `cargo test` / `cargo clippy` still pass.

## 5. Tasks

The integration branch was cut at c13b52b, before PR #60 and before the dialogs moved to `FloatingWindow`. It has to be rebased on `main` before the code task starts. `rebase_base` is refused before the first merge, so a small docs-only task goes first.

| Task | Owner | Depends on | Notes |
|---|---|---|---|
| T-0 Audit, docs only: `audit.md` lists every `confirm()` caller and every handler that clears `app.overlay` / `app.window` and then reads a value derived from it, read from `origin/main` with `git show` | back | — | Gate: merged, then the lead runs `rebase_base` onto `main` |
| T-1 Confirm dialog runs its callback, with the U3 tests through the real dialog | front | T-0 (and the rebase) | `ui/src/components/Dialogs.svelte`, plus any spot from the audit; `ui/test/`. `back` reviews |
