# UX brief: the Claude Code version warning

The user sees the warning in three places. Each must answer: **is this a problem
now?** (no, agents still run), **what is tested?** (the version), and **what do I
do?** (run `make contract`, read CONTRACT-TESTS.md). Reuse the existing components;
no new tokens.

## 1. Banner (`Banner.svelte`)

Today: `Check: claude 2.1.323 is newer than the last tested version: run the contract tests (R2)`.

- Title stays **Check:**, colour stays `--color-status-queued`.
- Text: the core's warning (spec §3.2), shown as is: the command and doc path
  render in mono (backticks in the string → `<span class="mono">`).
- New **Details** button (same as the Claude-update banner's) → Settings › Claude Code.
  Only on the version warning (matched by the prefix in api-contract §1).
- Dismissable as today.

## 2. Settings › Claude Code › Compatibility (`ClaudeSettings.svelte`)

When the installed version is newer than `tested_max`:

```
Compatibility
Supported    2.1.302 (minimum) – 2.1.302 (last tested)
             ▲ 2.1.323 is newer than the last tested version. Agents still run.
To verify    make contract                                   [copy on click]
             Runs the contract tests against this Claude Code (a few cents of
             plan quota). docs/app/CONTRACT-TESTS.md
```

- "To verify" is a `kv` row like the others. The command is a mono button with the
  "Copy" tooltip (as the path above), toast "Copied".
- The second line is muted. The doc path is plain mono text (the app has no
  repo checkout path to open it from).
- The same row also appears for the "latest is newer than the last tested version"
  note (before updating), with the copy "Updating is fine: agents still run. Verify
  after updating with `make contract`."
- Not shown when the version is within the range or too old (too old keeps its red note).

## 3. Setup pane (`SetupPane.svelte`)

Suffix after the version: ` (newer than the last tested version 2.1.302: agents still run; verify with make contract)`, `make contract` in mono.

## 4. Copy rules

No "R2", no "contract tests" without the command next to them. "Agents still run"
always appears with an untested warning.
