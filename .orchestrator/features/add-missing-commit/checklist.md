# add-missing-commits: verification checklist

One section per recovered commit. Run the "missing" command before porting (it must show a
diff) and the checks after. `origin/main` is where PR #78 landed. The source branches are
`orch/allow-multiple-prs/{integration,back-1,front-1}`.

Run everything from the repository root. Set `ORCH_TEST_TMP=$TMPDIR/t` for the Rust tests in
the sandbox. Socket/pty/FSEvent tests may fail locally: use `--no-fail-fast`, name them in
the report, and let CI judge them.

## 1. `06947016`: no lost PR when two issue PRs open at once (lead)

Touches `orch-store` and `agent-runtime`.

Missing from origin/main (must print a diff):

```sh
git diff origin/main orch/allow-multiple-prs/integration -- \
  crates/orch-store/src/teams.rs crates/orch-store/src/migrations.rs \
  crates/agent-runtime/src/pr_issue_tests.rs crates/agent-runtime/src/feature_prs_tests.rs
```

Checks:

```sh
cargo fmt --check
cargo clippy -p orch-store --all-targets -- -D warnings
cargo clippy -p agent-runtime --all-targets -- -D warnings
cargo test -p orch-store --no-fail-fast migrations
cargo test -p agent-runtime --no-fail-fast pr_issue_tests
cargo test -p agent-runtime --no-fail-fast feature_prs_tests
```

In particular `two_issues_get_their_own_prs_updates_and_the_run_ends_when_both_merge` must
pass, and the `migrations` stale-copy check must pass.

## 2. `ce70bdf2`: docs of issue_rows/row_ref, reuse row() helper in test (back)

Touches `agent-runtime` only (`crates/agent-runtime/src/issues.rs`).

Missing from origin/main (must print a diff):

```sh
git diff origin/main orch/allow-multiple-prs/back-1 -- crates/agent-runtime/src/issues.rs
```

Checks:

```sh
cargo fmt --check
cargo clippy -p agent-runtime --all-targets -- -D warnings
cargo test -p agent-runtime --no-fail-fast issues
```

## 3. `44143ed7`: issue chip sits next to the card title (front)

Touches the UI only (`ui/src/components/TaskIssueChip.svelte`, a comment change). Commands
come from `ui/package.json`: `lint` is `biome ci .`, `check` is `svelte-check --tsgo
--fail-on-warnings`, `test` is `vitest run`.

Missing from origin/main (must print a diff):

```sh
git diff origin/main orch/allow-multiple-prs/front-1 -- ui/src/components/TaskIssueChip.svelte
```

Checks (from `ui/`):

```sh
cd ui
bun run lint                      # biome ci .
bun run check                     # svelte-check --tsgo --tsconfig ./jsconfig.json --fail-on-warnings
bun run test -- TaskIssueChip     # vitest run TaskIssueChip
bun run test                      # full suite, once, before reporting
```

## Final, on the PR head

```sh
git diff origin/main HEAD --stat   # only the files listed above (plus .orchestrator/ docs)
cargo fmt --check
cargo clippy --workspace -- -D warnings
```
