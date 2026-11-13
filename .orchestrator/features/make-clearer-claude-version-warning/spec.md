# Make it clear how to address the Claude Code version warning

## 1. Problem

After a Claude Code update the app shows:

> Check: claude 2.1.323 is newer than the last tested version: run the contract tests (R2)

(also in the Setup pane, and as an amber note in Settings › Claude Code ›
Compatibility). Nothing tells the user what "the contract tests" are, where they
live or what to do once they pass. They don't exist as such:

- SPEC R2 / NFR-7 / FR-1201 / §6.4 promise "a contract test suite (R2) that
  replays the §6.4 checks after every Claude Code update". It was never built.
- What does exist: the P0 spike harness (`spikes/claude-cli`, throwaway) and two
  opt-in live tests, `crates/agent-runtime/tests/live_claude.rs` and
  `live_command.rs` (`ORCH_LIVE_CLAUDE=1 cargo test … -- --ignored`, uses plan
  quota). They cover most of §6.4 (hooks → state, statusline `rate_limits`,
  OTLP, sandboxed Bash, commit through orch-mcp, suspend/resume by session id,
  a typed slash command) but nothing names them "contract tests".
- The last tested version is a constant, `SUPPORTED_CLAUDE.tested_max` in
  `crates/agent-runtime/src/preflight.rs` (2.1.302, the P0 baseline). Nothing
  explains that clearing the warning means bumping it.
- "R2" is a spec risk id, meaningless in the UI.

## 2. Goal

One command runs the contract tests against the installed Claude Code. One doc
says what they check, what they cost and what to do when they pass or fail. The
warning names the tested version, says agents still run, and points to that
command and doc. When the tests pass, the version is recorded as tested and the
warning goes away.

## 3. Decisions

### 3.1 The contract test suite (back)

- **`make contract`** runs `scripts/contract.sh`. The script:
  1. prints the `claude` it tests (path and `claude --version`) and the current
     `tested_max`;
  2. runs the **no-quota checks** (new `crates/agent-runtime/tests/contract_cli.rs`,
     opt-in like the live tests): each flag the app passes (§6.3:
     `--settings`, `--mcp-config`, `--strict-mcp-config`, `--setting-sources`,
     `--append-system-prompt`, `--session-id`, `--resume`, `--fork-session`,
     `--permission-mode` with its five modes, `--model`, `--effort`, `-n/--name`)
     still appears in `claude --help`, and `claude auth status --json` still has
     `loggedIn`, `authMethod`, `subscriptionType`;
  3. runs the **live checks**: `live_claude` and `live_command` (haiku, a few
     cents of plan quota; the weekly guard above 90% stays). It says so and asks
     for confirmation first, unless `--yes` is passed. `--no-live` skips them;
     a run without them never records a version;
  4. prints a one-line summary per check (pass/fail) and, when every check passed,
     how to record the version.
- **`make contract-accept`** (`scripts/contract.sh --accept`): runs the suite,
  then, only when everything passed (live included), rewrites `tested_max` in
  `preflight.rs` to the installed version (and the "P0 baseline" comment, plus
  NFR-7's "last tested" line in SPEC). The user commits it like any change
  (a PR); the next build no longer warns. It never edits `min`.
- The checks keep failing loudly with the field or flag that changed, so a
  failure says what to fix in the app (§3.4 doc lists where each one is used).
- No change to CI: the suite needs a logged-in `claude` and plan quota.

**Why the source constant, not a local "accepted" file:** the version range is a
property of the code (hooks, flags, statusline parsing), and every build of the
same commit should agree. A local override would hide the warning on one Mac for a
build nobody tested. (Open for the user at spec approval: a "mark as tested on this
Mac" escape hatch, see §6.)

### 3.2 The warning text (back, one string)

The start-up warning (`runtime.rs`) and the live one (`setup.rs::version_warnings`)
become one function with this text:

> claude 2.1.323 is newer than the last tested version (2.1.302). Agents still run. To verify it, run `make contract` in an Orchestrator checkout (docs/app/CONTRACT-TESTS.md).

`is_runtime_version_warning` keeps matching it (prefix
`claude <v> is newer than the last tested version`). "R2" goes from every
user-facing string.

### 3.3 The UI (front, after ui)

See ux-brief.md. In short: the banner's version warning gets a **Details** button
(Settings › Claude Code), the Compatibility block gets a "To verify" row with the
command (mono, click to copy) and the doc path, and the Setup pane's suffix says
"newer than the last tested version (2.1.302): agents still run" with the same
pointer.

### 3.4 Docs (back)

- New `docs/app/CONTRACT-TESTS.md`: what the warning means; agents still run
  (unless strict); what the suite checks (table: check → app code that depends on
  it → test); cost and prerequisites (logged in, trusted scratch repo
  `/tmp/orch-live/demo`, `ORCH_LIVE_ANSWER_TRUST=1` the first time); how to run,
  read a failure, and accept a version; what to do on a failure (stay on the
  older version: `claude install <version>` or wait for a fix).
- `docs/app/RUNNING.md`, `crates/agent-runtime/README.md`, SPEC R2 / NFR-7 /
  FR-1201 / §6.4 link to it. The preflight.rs doc comment too.

## 4. Out of scope

- Running the suite from inside the app (it needs a source checkout and cargo).
- Running it in CI.
- Changing `min`, `strict`, or the agents' behaviour on an untested version.

## 5. Tasks

| Task | Node | Depends on |
|---|---|---|
| back-1: `scripts/contract.sh`, `make contract` / `contract-accept`, `contract_cli.rs`, the shared warning string, CONTRACT-TESTS.md and doc links | back | — |
| ui-1: Settings › Claude Code Compatibility block (untested state) and the banner with Details, in the Figcraft file; export to `design/` | ui | — |
| front-1: Banner Details button, Compatibility "To verify" row, Setup pane copy, tests | front | ui-1, back-1 (string) |

## 6. Open question for the user

Do you also want an in-app **"Mark 2.1.323 as tested on this Mac"** action,
stored locally, that hides the warning without a new build? Recommended: **no**
(see §3.1). Without it, the warning stays until a build with the bumped
`tested_max` is installed.

## 7. Acceptance

- `make contract` on a Mac with Claude Code installed runs and reports each check;
  `--no-live` uses no quota.
- `make contract-accept` after a full pass bumps `tested_max`; after a failure it
  changes nothing.
- The banner, Setup pane and Settings › Claude Code show the tested version, say
  agents still run, and point to `make contract` and CONTRACT-TESTS.md; no "R2".
- `make ci` passes.
