# Fix Linear Pull · API contract

No command, event or payload changes.

- `board_view`, `board_sync`, `linear_states`, `linear_teams`, `task_update`: same inputs and outputs. After this feature, `board_view` simply holds more Linear tasks: the open foreign issues, `backend: "linear"`, `external_id` = the issue identifier (`FUS-1503`).
- `board_sync`'s summary (`pushed`, `pulled`, `imported`, `updated`, `conflicts`, `errors`, `unmapped_states`) is unchanged. The first sync of a big team can report hundreds of `imported`.
- Front's only change is the copy in Project settings › Task backend (spec §3.3). It doesn't depend on back's task.
