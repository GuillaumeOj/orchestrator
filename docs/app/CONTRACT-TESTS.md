# Contract tests: checking a new Claude Code version

Orchestrator drives the real `claude` command: it passes flags, installs hooks and a
status line, and reads what they report. None of that is a documented, stable API, so
a Claude Code update can break it. The **contract tests** check that the installed
Claude Code still behaves the way the app relies on.

## What the warning means

After a Claude Code update the app can show:

> claude 2.1.323 is newer than the last tested version (2.1.302). Agents still run. To verify it, run `make contract` in an Orchestrator checkout (`docs/app/CONTRACT-TESTS.md`).

- **Agents still run.** The warning is information, not a block (unless the build is
  strict, then a newer version is refused).
- **It does not mean anything is broken.** It means nobody has run the checks below
  on this version yet.
- **It goes away** in a build whose `tested_max` (in
  `crates/agent-runtime/src/preflight.rs`) is at least the installed version. That is
  a source constant, so every build of the same commit agrees; there is no "mark as
  tested on this Mac" button.

You need an Orchestrator source checkout, Rust (`cargo`), and a logged-in `claude`.
If you run a released build and have no checkout, clone the repository first
(`git clone https://github.com/GuillaumeOj/orchestrator`), then run the commands below
from its root.

## Run them

```bash
make contract                    # checks, asks before using quota
make contract ARGS=--no-live     # only the checks that use no quota
make contract ARGS=--yes         # do not ask before the live checks
make contract-accept             # after a full pass: record the version
```

`make contract` runs `scripts/contract.sh`. It prints the `claude` it tests, its
version and the current `tested_max`, runs each check, and ends with one PASS/FAIL
line per check. The exit code is non-zero when any check failed.

**Cost.** The no-quota checks only run `claude --help` and `claude auth status`. The
live checks start real agents on `haiku` (a few cents of plan quota; the weekly guard
above 90% stays in force). The script asks first.

**First run.** The live checks use a scratch repository at `/tmp/orch-live/demo`
(or `ORCH_LIVE_REPO=<path>`). Claude Code asks once whether you trust it. Trust it
yourself, or set `ORCH_LIVE_ANSWER_TRUST=1` the first time and the test answers the
dialog for you. `ORCH_LIVE_CLAUDE_BIN=<path>` tests another `claude`.

## What is checked

| Check | App code that depends on it | Test |
|---|---|---|
| Every flag the app passes is in `claude --help`: `--settings`, `--mcp-config`, `--strict-mcp-config`, `--setting-sources`, `--append-system-prompt`, `--session-id`, `--resume`, `--fork-session`, `--permission-mode`, `--model`, `--effort`, `-n` (and `--add-dir`) | `compile.rs` builds the command line | `contract_cli`: `every_flag_the_app_passes_is_in_claude_help` |
| Each permission mode (`manual`, `acceptEdits`, `auto`, `dontAsk`, `plan`) is accepted by `--permission-mode` | `role.rs` `PermissionMode`, passed by `compile.rs` | `contract_cli`: `every_permission_mode_the_app_uses_is_listed` |
| `claude auth status --json` has `loggedIn`, `authMethod`, `subscriptionType` | Setup pane and the "Read-only" banner (`orch-app` `setup.rs`) | `contract_cli`: `auth_status_json_has_the_fields_the_app_reads` |
| Hook events reach the core and drive the agent state (working, waiting, idle) | `orch-hook`, the state machine | `live_claude` |
| The status line reports `rate_limits` | `orch-status`, the Governor's usage readings | `live_claude` |
| OTLP `api_request` events arrive at the local receiver | the OTLP receiver (usage and cost) | `live_claude` |
| The sandbox confines Bash to the worktree | per-agent settings from `compile.rs` | `live_claude` |
| A commit through `orch-mcp` works | `orch-mcp` (`commit`) | `live_claude` |
| Suspend and resume by session id (`--resume`) | the Runtime's suspend/resume | `live_claude` |
| A typed `/command` is delivered and runs | PTY delivery | `live_command` |

The flags come from `compile.rs` itself: add a flag there and the test checks it
without any change to the test.

## Reading a failure

A failing check names what changed: the flag that left `claude --help`, the field
missing from `claude auth status --json`, or the step of the live run that stopped.
Find that row in the table, fix the app code in the middle column, and run again.
Nothing is recorded after a failure.

If you cannot fix it now, stay on the version that passed: `claude install <version>`
(for example the current `tested_max`), or wait for an Orchestrator update. Agents
keep running on the newer version in the meantime, at your own risk.

## Record a version as tested

When every check passed, live ones included:

```bash
make contract-accept
```

This runs the suite again and, only after a full pass, rewrites `tested_max` in
`crates/agent-runtime/src/preflight.rs` to the installed version. It shows the diff.
Commit it and open a PR like any change; builds that include it stop warning about
that version. It never changes `min`, the oldest supported version. A run with
`--no-live` never records a version.

This suite is not run in CI: it needs a logged-in `claude` and plan quota.
