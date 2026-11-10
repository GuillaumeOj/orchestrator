// The static option lists of the Select pickers (design-notes §2.1, §2.1.1):
// one source per list, so two pickers of the same thing never disagree
// (§9.7: there were four model lists). Each list is `[{ value, label }]`,
// the shape `Select` takes.
import { TIERS as TIER_IDS } from './allocation.js';
import { COLUMNS } from './teams.js';

const plain = (ids) => ids.map((v) => ({ value: v, label: v }));

/** The model aliases Claude Code accepts (the core's estimator families). */
export const MODEL_ALIASES = ['opus', 'sonnet', 'haiku', 'fable'];

/** Required-value model pickers (Overwatch model, a downgrade target): the aliases only. */
export const MODEL_OPTIONS = plain(MODEL_ALIASES);

/** A downgrade goes to a cheaper model: opus, sonnet, haiku (no fable). */
export const DOWNGRADE_OPTIONS = MODEL_OPTIONS.filter((m) => m.value !== 'fable');

/** A model picker with no custom id (new session): default, then the aliases. */
export const MODEL_CHOICES = [{ value: '', label: 'default' }, ...MODEL_OPTIONS];

/** Optional model pickers that take a custom id (a role, a node): `MODEL_CHOICES`, then custom. */
export const MODELS = [...MODEL_CHOICES, { value: 'custom', label: 'custom…' }];

export const EFFORTS = [{ value: '', label: 'default' }, ...plain(['low', 'medium', 'high'])];

export const PERMISSION_MODES = [
  { value: '', label: 'default (ask)' },
  { value: 'acceptEdits', label: 'accept edits' },
  { value: 'plan', label: 'plan' },
  // What an autonomous role starts with (Dialogs `pickRole`).
  { value: 'auto', label: 'auto' },
];

/** Allocation tiers, highest first (`lib/allocation.js:TIERS` holds the ids). */
export const TIERS = plain(TIER_IDS);

export const REVIEW_MODES = [
  { value: 'self', label: 'self: /code-review, then /simplify' },
  { value: 'node', label: 'by the reviewer node' },
  { value: 'none', label: 'by the lead' },
];

export const PERMISSION_PROFILES = plain(['strict', 'gated', 'autonomous']);

export const BUDGET_PRIORITIES = plain(['low', 'normal', 'high', 'critical']);

/** Board columns (`lib/teams.js:COLUMNS`), e.g. a Linear state's column. */
export const BOARD_COLUMNS = COLUMNS.map((c) => ({ value: c.id, label: c.label }));
