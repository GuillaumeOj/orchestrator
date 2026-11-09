// Drives the Select the way a user does (T3 replaced every native <select>):
// the combobox button opens a listbox on the body; a row click picks.
import { fireEvent } from '@testing-library/svelte';
import { tick } from 'svelte';

/** The combobox button: `from` itself, or the one inside it (first, or `n`-th). */
export const combo = (from, n = 0) =>
  from.matches?.('[role="combobox"]') ? from : [...from.querySelectorAll('[role="combobox"]')][n];

/** The picked value (the hidden input every Select carries). */
export const picked = (from, n = 0) => combo(from, n).parentElement.querySelector('input[type="hidden"]').value;

/** The closed box's text. */
export const shown = (from, n = 0) => combo(from, n).querySelector('.val').textContent;

const rows = () => [...document.body.querySelectorAll('[role="option"]')];

/** Opens the list; returns the option labels, then closes it again (Esc keeps the value). */
export async function labels(from, n = 0) {
  const b = combo(from, n);
  await fireEvent.click(b);
  await tick();
  const out = rows().map((o) => o.querySelector('.lab').textContent);
  await fireEvent.keyDown(b, { key: 'Escape' });
  await tick();
  return out;
}

/** Opens the list and clicks the row labelled `label`. */
export async function pick(from, label, n = 0) {
  await fireEvent.click(combo(from, n));
  await tick();
  const row = rows().find((o) => o.querySelector('.lab').textContent === label);
  if (!row) throw new Error(`no option "${label}" in ${rows().map((o) => o.textContent)}`);
  await fireEvent.click(row);
  await tick();
}
