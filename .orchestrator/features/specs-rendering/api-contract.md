# Specs Rendering · API contract (core ↔ UI)

> Companion of [spec.md](spec.md). Back owns the Rust side, front the Svelte side. Back lands first; front can mock `feature_asset` in tests and code against this file meanwhile.

## 1. `feature_asset` (new Tauri command)

```js
invoke('feature_asset', { feature: '<feature id>', path: 'design/home.png' })
// → { mime: 'image/png', data_b64: '…', bytes: 48213 }
```

| Arg | Meaning |
|---|---|
| `feature` | Feature id (the same id `feature_doc` takes). |
| `path` | The image path **as written in the Markdown**, relative to `.orchestrator/features/<slug>/` (a leading `./` is allowed; front strips a `#…`/`?…` suffix and percent-decodes before calling). |

Behaviour (`agent-runtime`, next to `read_feature_doc`; thin wrappers in `orch-app` `teams.rs` / `commands.rs` / `lib.rs` like `feature_doc`):
- Reads from the feature's **integration worktree** (as `feature_doc` does).
- Refuses (error string, no read): an absolute path, any `..` component, a path whose canonical form is not under the feature folder, a symlink (`symlink_metadata` must be a plain file), an extension other than `png jpg jpeg gif webp svg` (case-insensitive), a file over `MAX_PREVIEW_BYTES` (`"too large (7.3 MB)"`).
- `mime` from the existing `orch_team::artifact::image_mime` helper (add `svg` → `image/svg+xml` to `PREVIEWABLE` if it lacks it).
- Errors are short, user-readable: `"not found"`, `"not an image"`, `"outside the feature folder"`, `"too large (… MB)"`.

## 2. `gate_detail` for `spec_approval`

Gains one field:

```json
{ "action": "spec_approval", "summary": "…", "docs": [{ "name": "spec.md", "text": "…" }],
  "feature_id": "<feature id>" }
```

`feature_id` is the id of the feature whose docs are listed: `request_spec_approval` stores it in the gate payload and `gate_detail` returns it. It is `null` for a spec gate created before this change; the gate then renders images as `image: <alt> (not available here)`.

## 3. `Markdown.svelte` props (front-internal, listed for the views that use it)

```svelte
<Markdown
  text={string}
  feature={featureId | null}      // enables relative images via feature_asset
  ondoc={(name, anchor) => void}  // enables links to sibling .md docs; absent → plain text
/>
```
