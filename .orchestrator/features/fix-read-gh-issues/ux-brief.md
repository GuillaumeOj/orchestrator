# Fix Read GitHub Issues From The Sandbox · UX brief

No design pass: two small additions that follow the existing components and tokens.

## 1. Launch dialog: GitHub issue field

- One optional text field, **GitHub issue**, above Title. Placeholder `#42 or issue URL`. Same `.field` style as Title.
- Under it, one line of preview, in the hint style:
  - while reading: `Reading #42…` (muted);
  - found: `#42 · <title> · open` (`closed` muted). A PR adds `· pull request`;
  - error: the core's message in the error colour. Launch stays enabled: the launch itself shows the error if the user goes on.
- The preview runs after 400 ms with no typing, or on blur. Only the latest answer is shown (an older reply that comes back late is dropped).
- A found issue fills **Title** only when Title is empty or still holds the title of a previous preview. Text the user typed is never overwritten.

## 2. Board: Launch a team…

- In a GitHub-backed card's menu, next to **Open on GitHub**: **Launch a team…**. It opens the Launch dialog for the card's project with the issue field set to `#<number>`, and the preview fills the title.
- No new key unless one is free in the Board's map (front's call; say which in the report).
