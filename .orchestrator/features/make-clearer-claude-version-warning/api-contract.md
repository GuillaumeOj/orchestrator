# API contract: the Claude Code version warning

## 1. The warning string

Both the start-up warning (`state().warnings`, from `Runtime::start`) and the
live one (`setup::version_warnings`) are built by one function,
`agent_runtime::preflight::untested_warning(v, tested_max)`:

```
claude <v> is newer than the last tested version (<tested_max>). Agents still run. To verify it, run `make contract` in an Orchestrator checkout (docs/app/CONTRACT-TESTS.md).
```

- Stable prefix for matching (UI and `is_runtime_version_warning`):
  `claude <v> is newer than the last tested version`.
- Backticks mark code; the UI renders them in mono.
- The "older than the supported minimum" warning is unchanged.

## 2. `claude` state object (Settings › Claude Code, Setup pane)

Unchanged fields: `version`, `untested`, `supported: { min, tested_max }`.
Added to `supported`:

```json
"verify": { "command": "make contract", "doc": "docs/app/CONTRACT-TESTS.md" }
```

Constants in the core (`preflight::VERIFY_COMMAND`, `VERIFY_DOC`), so the copy and
the script name live in one place. Always present when `supported` is.

## 3. Scripts

- `make contract` → `scripts/contract.sh [--no-live] [--yes]`: exit 0 when every
  check that ran passed.
- `make contract-accept` → `scripts/contract.sh --accept [--yes]`: same, then
  rewrites `tested_max` in `crates/agent-runtime/src/preflight.rs` only when every
  check, live included, passed. Prints the diff and the next step (commit, PR).
