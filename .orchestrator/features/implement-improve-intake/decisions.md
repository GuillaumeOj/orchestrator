# Implement Improve Intake · Decisions

| # | Date | Decision | Why |
|---|---|---|---|
| D1 | 2026-11-13 | Spec approved as written: §6 Q1–Q3 accepted (the improve-intake design merged into main as #67 first; the old intake_* / task-backend commands and the Board are removed without aliases; one PR at the end). | User approval of the spec gate. |
| D2 | 2026-11-13 | ux's gate-review leftovers settled by lead and written in the contract: `brief_prefill` owned by B3; ReadyFilter uses the `kind` tag and must match the tracker; `work_tree.after` only with one parent list; `work_node` `children` default false; multi-root `root_ref` = common parent; repo-docs intake over a remote backend → `mismatch`; `Closes owner/name#n` across repos and the core closes on merge when GitHub did not; team-task fallback to a top-level `orch` issue with "Sub-task of #n"; old commands stay callable until I1 removes them after F1–F5. | Each was a gap or ambiguity in the contract; settled now so back and front build the same thing. |
