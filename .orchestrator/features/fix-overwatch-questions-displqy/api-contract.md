# API contract: question cards

Only what changes. Everything else in the decision shape stays as it is.

## Decision `q-<n>` (kind `question`): `context`

| Case | `context` block types, in order |
|---|---|
| Question with options (`ask_user` with `options`, or `payload.questions` from `AskUserQuestion`) | `options`, then `last_message` if any |
| Question without options | `markdown` (the question), then `last_message` if any (unchanged) |

The leading `markdown` block is **removed** when an `options` block is present.

## `options` block

```json
{"type": "options", "questions": [
  {"question": "<Markdown text>", "header": "Auth" | null, "multi": false,
   "options": [{"label": "Clerk", "description": "hosted" | null}]}
]}
```

- Same shape as today. `questions[].question` is now **Markdown**, and the UI
  renders it as Markdown. For a single `ask_user` question it is the whole
  `payload.question`; for `AskUserQuestion` it is that question's own text.

## `decide` with action `answer` (UI → core): `message`

Unchanged in shape: any non-empty text (`"empty reply"` error otherwise). The
UI builds it as:

```
<choice of question 1>[, <another choice>]
<choice of question 2>
                                   ← blank line, only when both parts are present
<free text, trimmed>
```

- choices only: the first part (as today);
- text only: the text;
- both: choices, a blank line, the text.
