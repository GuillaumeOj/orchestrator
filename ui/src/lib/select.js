// Keyboard selection by id over a list (j/k): the id survives a re-sort or a
// poll that adds items, where an index would point at another row.

/** The id `delta` rows from `selectedId` (clamped); the first id when it is gone. */
export function move(items, selectedId, delta) {
  if (!items.length) return null;
  const i = items.findIndex((x) => x.id === selectedId);
  if (i < 0) return items[0].id;
  return items[Math.max(0, Math.min(items.length - 1, i + delta))].id;
}
