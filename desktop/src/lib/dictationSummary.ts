import type { DictationTiming } from "./dictationApi";

const RETENTION_MS = 7 * 24 * 60 * 60 * 1000;
const MAX_RECORDS = 300;
const INCOMPLETE = new Set([
  "pending",
  "cancelled",
  "no_audio",
  "empty",
  "asr_failed",
  "paste_failed",
  "paste_unconfirmed",
  "paste_skipped",
]);
const FALLBACK_REASONS = new Set([
  "cloud_paused",
  "cloud_cooldown",
  "cloud_paused_during_cleanup",
  "cleanup_timeout",
  "cleanup_request_failed",
  "cleanup_http_error",
  "cleanup_redirect_rejected",
  "invalid_response",
  "cleanup_response_too_large",
  "cleanup_storage_unavailable",
  "claude_key_unavailable",
  "missing_claude_api_key",
  "cloud_state_unavailable",
]);
const COMPLETED_OUTCOMES = new Set([
  "cleaned",
  "ready",
  "off",
  "paste_read_observed",
  "unicode_input_submitted",
  "unicode_input_submitted_copy_failed",
  ...FALLBACK_REASONS,
]);

function validDuration(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

function median(values: number[]): number {
  const sorted = [...values].sort((left, right) => left - right);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2
    ? sorted[middle]
    : (sorted[middle - 1] + sorted[middle]) / 2;
}

/** Only known software completion is a completed wait; screen paint is unmeasured. */
export function isCompletedTiming(row: DictationTiming): boolean {
  return (
    COMPLETED_OUTCOMES.has(row.outcome) &&
    [row.asr_ms, row.cleanup_ms, row.paste_ms, row.total_ms].every(
      validDuration,
    )
  );
}

export function summarizeResources(row: DictationTiming): {
  sampledWorkingSetBytes: number | null;
  processCpuMs: number | null;
  samples: number;
} {
  const snapshots = row.resource_snapshots ?? [];
  const memory = snapshots
    .map((snapshot) => snapshot.working_set_bytes)
    .filter(validDuration);
  const firstCpu = snapshots[0]?.process_cpu_ms;
  const lastCpu = snapshots[snapshots.length - 1]?.process_cpu_ms;
  return {
    sampledWorkingSetBytes: memory.length ? Math.max(...memory) : null,
    processCpuMs:
      snapshots.length >= 2 &&
      validDuration(firstCpu) &&
      validDuration(lastCpu) &&
      lastCpu >= firstCpu
        ? lastCpu - firstCpu
        : null,
    samples: snapshots.length,
  };
}

export function summarizeTimings(rows: DictationTiming[], now: number) {
  const records = rows
    .filter(
      (row) =>
        Number.isFinite(row.recorded_at_ms) &&
        Math.max(0, now - row.recorded_at_ms) < RETENTION_MS,
    )
    .sort((left, right) => right.recorded_at_ms - left.recorded_at_ms)
    .slice(0, MAX_RECORDS);
  const completed = records.filter(isCompletedTiming);
  const reasons = new Map<string, number>();
  for (const row of records) {
    // Legacy records stored the fallback only as outcome. A later insertion
    // failure can overwrite outcome, so newer records retain cleanup_reason.
    const reason =
      row.cleanup_reason?.trim() ||
      (FALLBACK_REASONS.has(row.outcome) ? row.outcome : null);
    if (
      row.mode === "claude" &&
      reason &&
      !INCOMPLETE.has(reason) &&
      !["cleaned", "ready", "off"].includes(reason)
    ) {
      reasons.set(reason, (reasons.get(reason) ?? 0) + 1);
    }
  }
  return {
    records,
    completed,
    cancelled: records.filter((row) => row.outcome === "cancelled").length,
    noAudio: records.filter((row) => row.outcome === "no_audio").length,
    unconfirmed: records.filter(
      (row) =>
        !isCompletedTiming(row) &&
        row.outcome !== "cancelled" &&
        row.outcome !== "no_audio",
    ).length,
    fallbackReasons: [...reasons]
      .map(([reason, count]) => ({ reason, count }))
      .sort(
        (left, right) =>
          right.count - left.count || left.reason.localeCompare(right.reason),
      ),
    medians: completed.length
      ? {
          asr: median(completed.map((row) => row.asr_ms)),
          cleanup: median(completed.map((row) => row.cleanup_ms)),
          paste: median(completed.map((row) => row.paste_ms as number)),
          total: median(completed.map((row) => row.total_ms)),
        }
      : null,
    slowest: [...completed]
      .sort((left, right) => right.total_ms - left.total_ms)
      .slice(0, 3),
    smallSample: completed.length < 5,
  };
}
