# Fix Linear Sync · Feature spec

> Feature `fix-linear-sync` · Integration branch `orch/fix-linear-sync/integration` · Backend only (`orch-tasks`) · no UI change, no API change

## 1. Problem

On a Linear-backed project (Fusily, today the only one), every Linear sync fails. Loading the team's projects and states in Project settings › Task backend fails too. The app shows Linear's error, **"Query too complex"**, and nothing is pulled or pushed.

## 2. Cause (HEAD 5882f8b)

Linear rejects any GraphQL query whose estimated complexity is over **10 000 points**. It estimates before running the query, so the size of the actual data doesn't matter. The documented rule:

- each scalar field: 0.1 point
- each object: 1 point
- a connection multiplies its children's points by its `first:` (or **50** when no `first:` is given)

`TEAM_QUERY` in `crates/orch-tasks/src/linear.rs` (operation `OrchTeam`) is over that limit:

```graphql
teams(filter: { key: { eq: $key } }) {          # no first: → × 50
  nodes { id key name
    states(first: 100)   { nodes { id name type position } }   # 100 × 1.4 = 140
    projects(first: 100) { nodes { id name } } } }             # 100 × 1.2 = 120
```

That is about 261 points per team × 50 teams (the default page), so about **13 000 points**. The filter returns one team at most, but Linear still counts 50. Every path that reads the team runs this query:

- `linear_states` (settings: the team's states and projects) → `team_states` → `team_info`
- the board sync and every issue write → `LinearIssues::team()` / `refresh_team()` (`list`, `create`, `update`, `state_for`, `label`)

So both the settings and the sync fail, which matches the report.

The other operations, estimated the same way:

| Operation | Estimate | Status |
|---|---|---|
| `OrchTeam` | ~13 000 | **over the limit** |
| `OrchIssues` (`issues(first: 25)` × `FIELDS`, with `labels` and `inverseRelations` at the default 50) | ~4 600 | under, but only by about 2×, and it grows with any field added to `FIELDS` |
| `OrchIssue`, `OrchIssueCreate`, `OrchIssueUpdate` (one `FIELDS`) | ~185 | ok |
| `OrchLabelFind` (`issueLabels(first: 50)`) | ~115 | ok |
| `OrchTeams` (`teams(first: 100)`) | ~130 | ok |
| Mutations without `FIELDS`, the Integrations check (`viewer`, `organization`) | < 10 | ok |

## 3. Fix

All of it is in `crates/orch-tasks/src/linear.rs`.

1. **`TEAM_QUERY`**: add `first: 1` to `teams(...)`. The key is unique, so this can't drop a team. The query drops to about 260 points.
2. **`FIELDS`**: give `labels` and `inverseRelations` an explicit `first: 50` (the same as today's default), so every connection the module sends has a visible bound and the page cost is plain to see. Keep `issues(first: 25)`. Nothing is lost: an issue with more than 50 labels or 50 blocking relations is out of scope, as it is today.
3. **Regression guard**: the fake (`fake::FakeLinear`) estimates each query it receives with the rule in §2 and answers the way Linear does when a query is over 10 000: `GqlError::Failed("Query too complex")`. The estimator is a small pure function under the `fake` module: a brace walk over the query text, where `first: N` on a field multiplies its sub-selection and a connection without `first:` multiplies by 50. With it, every existing Linear test (`orch-tasks` tests, `orch-app` `core.rs` Linear tests) also checks query cost. Add one unit test:
   - the estimate of each query the module sends is ≤ 10 000. Assert an actual margin: ≤ 5 000 per query, so `OrchIssues` keeps headroom.
   - the old `TEAM_QUERY` text (no `first:`) estimates over 10 000, which proves the estimator would catch this bug.
4. **Error text**: no change. `parse_answer` already passes Linear's message through (`Linear: Query too complex`). Once the queries are under the limit, it no longer shows up.

## 4. Out of scope

- Paging the team's states or projects past 100, and labels or relations past 50.
- Any UI change. The settings pane and the board already show the sync result and its errors.
- Reading Linear's actual complexity from the answer headers.

## 5. Acceptance

1. `cargo test -p orch-tasks` and the Linear tests in `orch-app` pass. The new estimator test fails on the old `TEAM_QUERY` and passes on the new one.
2. `cargo clippy` and `cargo fmt --check` are clean on the touched crate.
3. Manual check by the user on Fusily: Project settings › Task backend lists the team's states and projects, and a board sync finishes with "N pushed · N pulled · N conflicts", not "Query too complex". (The sandbox has neither the Linear API key nor access to api.linear.app, so the team can't run this check.)

## 6. Tasks

- **back**: §3 items 1–3 in `crates/orch-tasks/src/linear.rs`, with the tests. One task.
- **front**: nothing to build. Reviews back's change when back asks.
