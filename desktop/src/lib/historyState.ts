import type { HistoryEntry, HistoryUpdatePayload } from "../bindings";

export function applyHistoryEvent(
  entries: HistoryEntry[],
  event: HistoryUpdatePayload,
): HistoryEntry[] {
  if (event.action === "deleted")
    return entries.filter((row) => row.id !== event.id);
  if (event.action === "toggled")
    return entries.map((row) =>
      row.id === event.id ? { ...row, saved: !row.saved } : row,
    );
  if (event.action === "updated")
    return entries.map((row) =>
      row.id === event.entry.id ? event.entry : row,
    );
  return [
    event.entry,
    ...entries.filter((row) => row.id !== event.entry.id),
  ].sort((a, b) => b.id - a.id);
}
export function mergeHistorySnapshot(
  current: HistoryEntry[],
  snapshot: HistoryEntry[],
  events: HistoryUpdatePayload[],
  replace: boolean,
): HistoryEntry[] {
  const page = events.reduce(applyHistoryEvent, snapshot);
  if (replace) return page.sort((a, b) => b.id - a.id);
  const byId = new Map<number, HistoryEntry>();
  for (const row of [...page, ...current]) byId.set(row.id, row);
  // Current rows already contain these events; only tombstones must also remove
  // rows introduced by the older page. Replaying a toggle would undo it.
  const deleted = new Set<number>();
  for (const event of events) {
    if (event.action === "deleted") deleted.add(event.id);
    else if (event.action === "added") deleted.delete(event.entry.id);
  }
  for (const id of deleted) byId.delete(id);
  return [...byId.values()].sort((a, b) => b.id - a.id);
}
