# Fix Write Back on GitHub · Feature spec

> Feature `fix-write-back-github` · Integration branch `orch/fix-write-back-github/integration` · Team: lead, back, front.
> Companion: [api-contract.md](api-contract.md) (error shape, new commands, events).
> Code base: **origin/main** (#80). The integration branch starts at #65, before write-back landed (#7x); it is
> rebased on main before any code task starts.

## 1. Problem

The user's project (a GitHub tracker with a Projects v2 board) shows **"write-back failed"** on its Work view items, and
the overwatch decision *Write-back is failing* says:

> gh needs the read:project scope: run `gh auth refresh -s read:project`. Fix it in the tracker settings or turn write-back off.

Nothing in Project settings › Tracker lets the user fix it. Running the command does not help either.

### 1.1 Root cause (read in the code on main)

1. **Wrong scope, wrong advice.** With a board set, write-back's *Started* op moves the item on the board
   (`orch-tasks/src/writeback.rs` `gh_board_status`: `addProjectV2ItemById`, `updateProjectV2ItemFieldValue`). Those
   mutations need the **`project`** scope (read-write). `read:project` only reads. GitHub answers:
   *"… The 'addProjectV2ItemById' field requires one of the following scopes: ['project'], but your token has only been
   granted the: ['gist', 'read:org', 'read:project', 'repo'] scopes."*
   `orch-tasks/src/intake.rs` `gh_error` sees `read:project` / `required scopes` in that text, classifies it `gh_scope`
   and replaces the message with the fixed read advice (`gh auth refresh -s read:project`). The user may already have
   `read:project`; the advice can never fix a write.
2. **No fix in settings.** The only `gh_scope` UI in Tracker settings is under the board picker (reading the board,
   `optsError`) and in the Preview error. The Write back block shows no error, no state of the gh login, no way to
   grant the scope and no way to retry. The decision's *Open settings* lands on a block that says nothing about the
   failure.
3. **Failures stay after a fix.** A node keeps `writeback.status = failed` until someone presses Retry on that one node;
   the `writeback_failing` decision stays open. After a fix the Work view still shows every warning.

## 2. Goals

1. A scope error names the scope GitHub asked for: **`project`** for board writes, `read:project` for board reads, and
   the matching command `gh auth refresh -h github.com -s <scope>`.
2. Project settings › Tracker › Write back shows **what is wrong and fixes it in place**:
   - the gh access the tracker needs vs what the login has (checked with `gh auth status`);
   - **Grant access**: the app runs the `gh auth refresh` device flow itself and shows the one-time code + *Open GitHub*;
   - **Copy command** as the fallback when the app cannot run it (no gh, a token from `GH_TOKEN`, gh refuses non-interactive);
   - **Retry failed writes (n)**: retries every failed write-back of the project.
3. Once access is granted, the failed writes are retried automatically; when none is left failing, the
   `writeback_failing` decision is resolved and the Work view warnings go away.
4. The decision card's text and the Work view's failed write-back (details history line, feature source header) say
   the right fix and link to the Write back block.

## 3. Non-goals

- No new write-back modes (e.g. "label instead of board status" while a board is set). *Turn off write-back* stays the
  other way out.
- Linear write-back is unchanged (its `linear_write` errors keep their current copy).
- No change to which ops write what (labels, comments, sub-issues, close).
- GitHub Enterprise hosts: the host is `github.com` (as everywhere else in the tracker today).
- Findings on gh's behaviour are in [gh-notes.md](gh-notes.md) (T-450); the implementation follows them.

## 4. Behaviour

### 4.1 Scope errors (back)

- `gh_error` keeps `gh_scope` as the code but parses the scope GitHub names (`requires one of the following scopes:
  ['x', …]` → first `x`). When the text names none: `read:project` on a read, `project` on a write (`gh_write_error`).
  A text that merely *lists* `read:project` among the granted scopes is not a reason to say `read:project`.
- The `SourceError` carries `scope` (api-contract §1); its `message` is
  `gh needs the <scope> scope: run \`gh auth refresh -h github.com -s <scope>\``.
- The write-back node status (`WorkNode.writeback`, `run.source.writeback`) carries `code` and `scope`. The stored
  setting has `code` already, but `wb_statuses` drops it (and `op`): it passes them through, and `wb_record` stores `scope`.
- The decision text becomes `"<message>. Fix it in Project settings › Tracker or turn write-back off."`.

### 4.2 GitHub access check (back)

`tracker_gh_access(project)` (api-contract §2) reads `gh auth status` (`Token scopes: …` line) and answers which scopes
the project's tracker needs (`read:project` when a board is set; `project` when write-back is on with a board), which
the login has, and which are missing. `granted: null` when gh does not list scopes (fine-grained token, `GH_TOKEN`
env): the UI then says it cannot check, and offers Retry failed writes (and Copy command when a failure is `gh_scope`).
`gh auth status` also fails offline: that is `offline`, not logged out.

### 4.3 Grant access (back)

`gh_grant_scope(scope)` runs `gh auth refresh -h github.com -s <scope>` with no TTY (stdin closed), reads the one-time
code and the device URL from its output, answers `{code, url}` at once, and posts `gh.grant` events as it ends
(api-contract §3). One grant at a time (a second call while one runs answers the running one). Timeout 15 min, then
the process is killed and `failed` posted. `scope` is validated (`project` | `read:project`). On `done` the core retries
the failed writes of every GitHub-tracker project (§4.4); the event reports it by project.

**Back must verify on the installed gh** that a non-TTY `gh auth refresh -h github.com -s project` prints the code and
polls without waiting for Enter. If it does not, `gh_grant_scope` answers `{code: "interactive_only"}` and the UI shows
Copy command only. `gh auth refresh -s` adds scopes: existing ones are kept.

### 4.4 Retry all (back)

`writeback_retry_all(project)` runs `writeback_retry` for every node of the project whose status is `failed`, in node
order, and answers counts. A success resets the failure count (already). When none is left failing, the open
`writeback_failing` decision of the project is resolved (closed as done, not as answered by the user), also when no
node was left to retry. A single `writeback_retry` that leaves no failed node resolves it too (today only Retry on every
node by hand, then nothing, closes the loop: the decision stays open).

### 4.5 Tracker settings › Write back block (front)

Shown when the tracker is GitHub and write-back is on. Under the switch, before the option rows, an **access row**:

| State | Copy | Actions |
|---|---|---|
| checking | "Checking GitHub access…" | — |
| ok, nothing failing | "GitHub access OK (<the scopes in `needed`>)." muted | — |
| missing scope | ⚠ "GitHub needs the `project` scope to move items on the board." | **Grant access** · Copy command |
| granting | "Enter **XXXX-XXXX** on GitHub." + code copy | **Open GitHub** · Cancel |
| granted | "Access granted. Retrying n failed writes…" → result | — |
| unknown (`granted: null`) | "Orchestrator can't check this gh login's scopes." muted | Copy command when a failure is `gh_scope` |
| failed writes (any state) | "n write-backs failed: <last message>." | **Retry failed writes** |
| gh missing / not logged in | existing Integrations copy | Open Integrations |

- *Grant access* calls `gh_grant_scope`, shows the code, *Open GitHub* opens the URL in the browser and copies the code.
- On `gh.grant` `done`: re-check access, show the retry result; on `failed`: show its message + Copy command.
- The block re-checks access when the section opens and when *Open settings* of the decision brings it into view.
- `GH_SCOPE_COMMAND` becomes `ghScopeCommand(scope)`; every existing `gh_scope` copy (board picker, Preview error,
  intake errors) uses the scope of the error (default `read:project` where the error has none).

### 4.6 Work view and feature header (front)

- WorkDetails history line `writeback_failed` and SourceHeader's failed write-back: when the status code is
  `gh_scope`, add **Fix** (opens Project settings › Tracker at the Write back block) next to Retry.
- The tail "write-back failed" in the Work list keeps its text; its tooltip shows the status message.

## 5. Acceptance

1. Unit: GitHub's real insufficient-scopes text for `addProjectV2ItemById` (granted list holding `read:project`) →
   `gh_scope`, `scope = "project"`, message with `-s project`. A read text naming `read:project` as required → `read:project`.
2. Unit: `gh auth status` outputs (classic token with scopes; no scopes line; logged out) → `tracker_gh_access` results.
3. Unit (fake gh): `gh_grant_scope` parses code + URL, posts `done` / `failed`; `done` triggers retry-all.
4. Runtime test: three failed board writes → decision open; `writeback_retry_all` with gh now answering → statuses `ok`,
   failure count 0, decision resolved, Work node `writeback.status = ok`.
5. UI tests: access row states of §4.5 (missing → Grant → code shown → done → retried), Copy command copies
   `gh auth refresh -h github.com -s project`, Fix link in WorkDetails / SourceHeader opens the Write back block.
6. `make test`, `make clippy`, `bun run check` / UI tests pass on the integration branch.

## 6. Tasks (after approval)

0. `lead`: rebase the integration branch on main (gate task, see memory "rebase base before tasks").
1. `back` (crates/orch-tasks, crates/agent-runtime, crates/orch-app wiring): §4.1–4.4 + tests.
2. `front` (ui/): §4.5–4.6 + tests, against the contract (can start on the contract; depends on back for the end-to-end check).
