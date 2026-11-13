# Fix GitHub Issue Pull · API contract

One addition to the intake source status, the object the Mission lane already gets per source (`agent-runtime/src/intake.rs`, the `count` / `more` / `hidden` / `error` / `url` object).

| Field | Type | Meaning |
|---|---|---|
| `partial` | bool, default `false` | The last refresh stopped at the read bound (20 pages, 2,000 raw items) before the end of the source: `more` counts only what was read. |

- `false` everywhere else: no source, an error, a refresh that read the whole source, the preview.
- `intake_preview` carries `partial` in its `preview` object, with the same meaning.
- Front: the "+ n more in <source>" line reads **"+ n+ more in <source>"** when `partial` is true. When `more` is 0 and `partial` is true, it reads "+ more in <source>". Nothing else changes.
