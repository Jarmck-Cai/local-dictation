import type { HistoryEntry } from "../bindings";
import type { DictationTiming } from "./dictationApi";
import { isCompletedTiming } from "./dictationSummary";

export const USAGE_RETENTION_MS = 7 * 24 * 60 * 60 * 1000;
export const USAGE_MAX_RECORDS = 300;

export const historyFinalText = (entry: HistoryEntry): string =>
  entry.post_processed_text?.trim()
    ? entry.post_processed_text
    : entry.transcription_text;

const inScope = (timestamp: number, now: number) =>
  Number.isFinite(timestamp) &&
  timestamp <= now &&
  now - timestamp < USAGE_RETENTION_MS;

/** Generated output is measurable here; the target app's actual text is not. */
export function summarizeUsage(
  entries: HistoryEntry[],
  metrics: DictationTiming[],
  now: number,
) {
  const seen = new Set<number>();
  const records = entries
    .filter((entry) => {
      if (seen.has(entry.id) || !inScope(entry.timestamp * 1000, now))
        return false;
      seen.add(entry.id);
      return true;
    })
    .sort(
      (left, right) => right.timestamp - left.timestamp || right.id - left.id,
    )
    .slice(0, USAGE_MAX_RECORDS);
  const timings = new Map<number, DictationTiming>();
  for (const metric of [...metrics].sort(
    (a, b) => b.recorded_at_ms - a.recorded_at_ms,
  )) {
    if (
      metric.history_id !== null &&
      !timings.has(metric.history_id) &&
      inScope(metric.recorded_at_ms, now)
    ) {
      timings.set(metric.history_id, metric);
    }
  }
  let transcriptions = 0;
  let characters = 0;
  let estimatedSamples = 0;
  let savedMs = 0;
  for (const entry of records) {
    const text = historyFinalText(entry);
    const count = Array.from(text.replace(/\s/gu, "")).length;
    if (!count) continue;
    transcriptions += 1;
    characters += count;
    const timing = timings.get(entry.id);
    if (
      !timing ||
      !isCompletedTiming(timing) ||
      typeof timing.recording_ms !== "number" ||
      !Number.isFinite(timing.recording_ms) ||
      timing.recording_ms < 0
    )
      continue;
    const han = text.match(/\p{Script=Han}/gu)?.length ?? 0;
    const english =
      text.match(/\p{Script=Latin}+(?:['’]\p{Script=Latin}+)*/gu)?.length ?? 0;
    // Punctuation, digits and other scripts count as generated characters but
    // receive no invented typing rate in the conservative time estimate.
    const referenceMs = (han / 60 + english / 40) * 60_000;
    estimatedSamples += 1;
    savedMs += Math.max(0, referenceMs - timing.recording_ms - timing.total_ms);
  }
  return { records, transcriptions, characters, estimatedSamples, savedMs };
}
