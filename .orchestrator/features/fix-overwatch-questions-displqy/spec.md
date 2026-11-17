# Fix how the Overwatch displays questions

## The problem

1. **The question shows twice.** A question card (kind `question`, id `q-<n>`)
   in the Overwatch chat column gets two context blocks from the core
   (`crates/agent-runtime/src/decision.rs`, the `("question", _)` arm):
   - a `markdown` block with the question text (`payload.question`), rendered
     with its formatting;
   - an `options` block, which carries every question's text again, which
     `ui/src/components/blocks/OptionsBlock.svelte` shows as plain bold text
     (`<b>{q.question}</b>`), with no Markdown.

   For an `ask_user` call with options, `payload.question` is the whole
   question, so the card shows it twice. For a forwarded `AskUserQuestion`,
   `payload.question` is every question with its options as a list
   (`questions.rs::ask_user_question_text`), so the card shows the options twice too.

2. **There is nowhere to type an answer of your own.** When a question has
   options, the card only offers radios or checkboxes. An option such as
   "Something else, please provide the information" (or Claude Code's own
   "Other") can be picked, but there is no field to write that information.
   The user has to send a second message through the Overwatch.

The example in the brief (a CI question about PR #415, a PR of another
project: `#415` is not an issue of this repository) is a long Markdown
question with options. It is the test case for both fixes.

## What changes

### 1. The question is shown once, with its formatting (back + front)

- **Core:** a question card with an `options` block has no leading `markdown`
  block. Its context is `options` then `last_message` when there is one. A
  question without options keeps its `markdown` block, as it does today.
- **UI:** `OptionsBlock` renders each `questions[].question` with the
  `Markdown` component (as `MarkdownBlock` does), not as `<b>` text. The
  legend ("Question", "Question 2 of 3 · Auth", "· Pick any") does not change.
- The card title (`cut(q, 80)`, "… · N questions") does not change.

### 2. A free-text field on every live question card that has options (front)

- Under the options, a live card (not a read-only expanded line) shows a
  textarea, labelled "Your answer", 2 rows, with the placeholder
  `Add details or answer in your own words · ⏎ to send`.
- **Answer** is enabled, and gets its primary style, when every question has a
  choice **or** the text field is not empty. Otherwise it stays disabled, and
  ⏎ on the card shows the toast "Pick an answer for every question" as it does today.
- In the field, ⏎ answers (same path as the Answer button) and Shift+⏎ adds a
  new line. Esc leaves the field and puts the focus back on the card. The
  field is a typing target, so the chat's single-key shortcuts do not fire
  while you type in it (already true for textareas).
- Text in the field holds the card open, the same way the reply textarea
  does (`onhold`), so the card does not collapse while you type.
- The message sent (`answer` action, its `message`): the choices as today
  (`answerText`: one line per question, multi choices joined with ", "), then a
  blank line, then the text. With no choice it is only the text. With no text
  it is exactly what is sent today. Leading and trailing whitespace of the text is trimmed.
- The text is per card, and is cleared once the answer is sent (or the card leaves).
- Questions without options do not change: Answer opens the reply textarea.

## Out of scope

- The ask_user / AskUserQuestion tools and how the agent receives the answer.
  The core already takes any text as the answer message.
- Labelling each choice line with its question header in the message sent.
- The flaky UI tests named in the example question (CoordinatesPanel, dialog
  timeouts): that is a separate issue.

## Tasks (planned)

- **back:** `decision.rs`: no `markdown` block on a question card that has an
  `options` block; update `decision/tests.rs`
  (`a_question_with_several_questions_lists_them_all` expects
  `["options", "last_message"]`), and add a test for a single `ask_user`
  question with options (context is `["options"]`, its question is the full text).
- **front:** `OptionsBlock` renders the question as Markdown; the free-text
  field in `DecisionCard` and its wiring into the `answer` intent in
  `ChatColumn` (`choices` + text → message); tests in `ui/test`
  (`decisions.test.js` and the chat fixture) for: the question is shown once,
  formatted; Answer is enabled by text alone; the message sent with choices +
  text, text alone, choices alone; ⏎ and Shift+⏎ in the field; the field is
  absent on a read-only line. The front task does not depend on the back task
  (the UI test fixtures carry their own decisions), and the two touch disjoint files.

## Acceptance

- The brief's CI question, asked with `ask_user` and options, shows its text
  once, formatted (paragraphs, inline code), above its options.
- A forwarded `AskUserQuestion` with 2 questions shows each question once,
  with its options once.
- Picking "Something else…" and typing details, then ⏎, sends both in one
  answer; no second message is needed.
- `cargo test -p agent-runtime` and the UI tests (`bun test` in `ui/`) pass.
