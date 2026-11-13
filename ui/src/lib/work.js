// Entry to the Work browser (F3 implements it). Until it lands every caller (Feature "Show in Work",
// palette "Work: <project>") goes through this one function, so F3 replaces only the body.
import { toast } from './app.svelte.js';

/**
 * Opens Work on `projects` (ids; null = all), revealing `reveal` (a WorkNode id) when given.
 * @param {{ projects?: string[] | null, reveal?: string | null, find?: boolean }} [scope]
 */
export function openWork(scope = {}) {
  toast(`Work browser: not available yet${scope.reveal ? ` (item ${scope.reveal})` : ''}`);
}
