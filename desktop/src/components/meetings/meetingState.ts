import type {
  MeetingDevice,
  MeetingExportFormat,
  MeetingExportLanguage,
  MeetingsTransport,
  MeetingStartOptions,
  MeetingSummary,
  MeetingTranscript,
} from "../../lib/meetings";

export type MeetingError =
  | "generic"
  | "busy"
  | "devices"
  | "capture"
  | "storage"
  | "model"
  | "modelUnsupported"
  | "processing"
  | "export"
  | "missing"
  | "unsupported";

const errorCodes: Record<string, MeetingError> = {
  meeting_active: "busy",
  meeting_busy: "busy",
  meeting_dictation_busy: "busy",
  meeting_processing: "busy",
  meeting_recording: "busy",
  meeting_device_required: "devices",
  meeting_device_unavailable: "devices",
  meeting_capture_device_inactive: "devices",
  meeting_capture_device_id_invalid: "devices",
  meeting_capture_device_id_null: "devices",
  meeting_capture_device_name_invalid: "devices",
  meeting_capture_device_name_null: "devices",
  meeting_capture_device_wrong_direction: "devices",
  meeting_capture_format_invalid: "devices",
  meeting_capture_format_null: "devices",
  meeting_capture_format_unsupported: "devices",
  meeting_capture_microphone_stalled: "capture",
  meeting_capture_health_gap: "capture",
  meeting_capture_queue_full: "capture",
  meeting_capture_initialize_timeout: "capture",
  meeting_capture_start_timeout: "capture",
  meeting_capture_commit_timeout: "capture",
  meeting_capture_stop_failed: "capture",
  meeting_capture_thread_panicked: "capture",
  meeting_capture_consumer_gone: "capture",
  meeting_capture_drain_limit: "capture",
  meeting_capture_event_wait_failed: "capture",
  meeting_capture_interval_invalid: "capture",
  meeting_recording_panicked: "capture",
  meeting_stop_timeout: "capture",
  meeting_io: "storage",
  meeting_storage_unavailable: "storage",
  meeting_writer_failed: "storage",
  meeting_atomic_replace: "storage",
  meeting_chunk: "storage",
  meeting_manifest: "storage",
  meeting_transcript: "storage",
  meeting_model_unavailable: "model",
  meeting_model_not_loaded: "model",
  meeting_model_missing: "model",
  meeting_model_unsupported: "modelUnsupported",
  meeting_vad_model_missing: "model",
  meeting_vad_init: "model",
  meeting_asr: "model",
  meeting_transcription_failed: "model",
  meeting_vad_process: "processing",
  meeting_processing_clock: "processing",
  meeting_processing_epoch: "processing",
  meeting_processing_interrupted: "processing",
  meeting_processing_panicked: "processing",
  meeting_transcript_missing: "processing",
  meeting_apm_init: "processing",
  meeting_apm_failed: "processing",
  meeting_apm_frame: "processing",
  meeting_apm_sample: "processing",
  meeting_apm_process: "processing",
  meeting_apm_reset: "processing",
  meeting_export: "export",
  meeting_export_destination: "export",
  meeting_export_format: "export",
  meeting_export_io: "export",
  meeting_path: "export",
  meeting_link: "export",
  meeting_id: "missing",
  meeting_not_found: "missing",
  meeting_capture_platform_unsupported: "unsupported",
  meeting_unsupported: "unsupported",
};

/** Only known static categories cross into user-facing copy. */
export function meetingErrorKey(error: unknown): MeetingError {
  return typeof error === "string" &&
    Object.prototype.hasOwnProperty.call(errorCodes, error)
    ? errorCodes[error]
    : "generic";
}

export type CaptureReason = {
  reason:
    | "stateSave"
    | "fileAccess"
    | "device"
    | "queue"
    | "capture"
    | "storage"
    | "unknown";
  code?: string;
};
export function meetingCaptureReason(
  meeting: MeetingSummary,
): CaptureReason | null {
  const cause =
    meeting.capture_error ??
    (meeting.status === "interrupted" ? meeting.error : null);
  if (!cause && meeting.status !== "interrupted") return null;
  const accessCodes = [
    "meeting_atomic_replace_win32_5",
    "meeting_atomic_replace_win32_32",
    "meeting_atomic_replace_win32_33",
  ];
  if (cause && accessCodes.includes(cause))
    return { reason: "fileAccess", code: cause };
  const stateCodes = [
    "meeting_atomic_replace",
    "meeting_atomic_replace_exists",
    "meeting_atomic_replace_win32_2",
    "meeting_atomic_replace_win32_3",
    "meeting_atomic_replace_win32_112",
    "meeting_atomic_replace_win32_1175",
    "meeting_atomic_replace_win32_1176",
    "meeting_atomic_replace_win32_1177",
  ];
  if (cause && stateCodes.includes(cause))
    return { reason: "stateSave", code: cause };
  if (cause === "meeting_capture_queue_full")
    return { reason: "queue", code: cause };
  if (cause === "meeting_capture_device_removed")
    return { reason: "device", code: cause };
  const category = meetingErrorKey(cause);
  const reason =
    category === "devices"
      ? "device"
      : category === "capture"
        ? "capture"
        : category === "storage"
          ? "storage"
          : "unknown";
  return reason === "unknown"
    ? { reason }
    : { reason, code: cause ?? undefined };
}

export type MeetingWarningKey =
  | "meetings.warnings"
  | "meetings.warningIncompleteRecording"
  | "meetings.warningAttributionExperimental"
  | "meetings.warningInputClamped";

const warningCodes: Record<string, MeetingWarningKey | null> = {
  meeting_attribution_experimental: "meetings.warningAttributionExperimental",
  meeting_alignment_incomplete_recording: "meetings.warningIncompleteRecording",
  meeting_apm_input_clamped: "meetings.warningInputClamped",
  // These observations have already passed native clock integrity validation.
  meeting_alignment_clock_timestamp_jitter: null,
  meeting_alignment_initial_discontinuity: null,
  meeting_alignment_remote_resume_discontinuity: null,
  meeting_alignment_remote_no_playback: null,
  meeting_alignment_single_anchor_nominal_rate: null,
  meeting_alignment_short_span_nominal_rate: null,
  meeting_alignment_boundary_padding: null,
};

/** Preserve recording facts after transcription completes; never display raw codes. */
export function meetingWarningKeys(
  warnings: readonly string[],
): MeetingWarningKey[] {
  const keys = new Set<MeetingWarningKey>();
  for (const code of warnings) {
    const key = Object.prototype.hasOwnProperty.call(warningCodes, code)
      ? warningCodes[code]
      : "meetings.warnings";
    if (key) keys.add(key);
  }
  return [...keys];
}

export const isMeetingActive = (meeting: MeetingSummary | null | undefined) =>
  !!meeting && ["recording", "paused", "processing"].includes(meeting.status);

export const canTranscribeMeeting = (meeting: MeetingSummary) =>
  ["recorded", "interrupted", "cancelled", "failed"].includes(meeting.status);

export type MeetingOperation =
  | "start"
  | "finish"
  | "transcribe"
  | "cancel"
  | "delete"
  | "export";

// Native cancellation is cooperative. Keep the request with its meeting job
// across page remounts; a Processing acknowledgement is not completion.
const cancellationRequests = new Map<string, symbol>();
// One webview belongs to one native app run. Preserve these small snapshots
// across page remounts so a delayed event cannot revive an earlier job.
const latestSummaries = new Map<string, MeetingSummary>();
// Keep the global activity floor after terminal/idle snapshots, even when no
// meeting is active. History rows still use their independent per-ID versions.
let activityRevision = 0;
let latestActivity: MeetingSummary | null = null;
const summaryRevision = (summary: MeetingSummary) => summary.revision ?? 0;
const isSummaryStale = (summary: MeetingSummary) =>
  summaryRevision(summary) <
  summaryRevision(latestSummaries.get(summary.id) ?? summary);
function newestSummary(summary: MeetingSummary) {
  const known = latestSummaries.get(summary.id);
  if (known && summaryRevision(known) > summaryRevision(summary)) return known;
  latestSummaries.set(summary.id, summary);
  return summary;
}
const isProcessingTerminal = (summary: MeetingSummary) =>
  ["cancelled", "complete", "failed"].includes(summary.status);

function activeAfterSummary(
  active: MeetingSummary | null,
  summary: MeetingSummary,
) {
  const revision = summaryRevision(summary);
  if (revision > 0 || activityRevision > 0) {
    if (revision > activityRevision) {
      if (
        latestActivity &&
        (latestActivity.id !== summary.id || !isMeetingActive(summary))
      )
        cancellationRequests.delete(latestActivity.id);
      activityRevision = revision;
      latestActivity = isMeetingActive(summary) ? summary : null;
    }
    return latestActivity;
  }
  // Unversioned fixtures retain the original per-ID activity semantics.
  if (!isMeetingActive(summary))
    return active?.id === summary.id ? null : active;
  return summary;
}

export interface MeetingsSnapshot {
  devices: MeetingDevice[];
  devicesLoading: boolean;
  devicesError: MeetingError | null;
  meetings: MeetingSummary[];
  listLoading: boolean;
  listError: MeetingError | null;
  currentError: MeetingError | null;
  currentLoading: boolean;
  selectedId: string | null;
  transcript: MeetingTranscript | null;
  transcriptLoading: boolean;
  transcriptError: MeetingError | null;
  active: MeetingSummary | null;
  busy: MeetingOperation | null;
  cancellingId: string | null;
  error: MeetingError | null;
  exported: boolean;
}

export const emptyMeetingsSnapshot = (): MeetingsSnapshot => ({
  devices: [],
  devicesLoading: false,
  devicesError: null,
  meetings: [],
  listLoading: false,
  listError: null,
  currentError: null,
  currentLoading: true,
  selectedId: null,
  transcript: null,
  transcriptLoading: false,
  transcriptError: null,
  active: null,
  busy: null,
  cancellingId: null,
  error: null,
  exported: false,
});

function replaceSummary(rows: MeetingSummary[], meeting: MeetingSummary) {
  return [meeting, ...rows.filter((row) => row.id !== meeting.id)].sort(
    (a, b) => b.created_at_ms - a.created_at_ms || a.id.localeCompare(b.id),
  );
}

/** Owns asynchronous UI reads and actions, with native capture behind a transport. */
export class MeetingsState {
  snapshot = emptyMeetingsSnapshot();
  private alive = true;
  private revision = 0;
  private listTicket = 0;
  private deviceTicket = 0;
  private transcriptTicket = 0;
  private currentTicket = 0;
  private polling = false;
  private changes = new Map<
    string,
    { revision: number; summary: MeetingSummary | null }
  >();

  constructor(
    private api: MeetingsTransport,
    private changed: (snapshot: MeetingsSnapshot) => void,
  ) {}

  private update(next: Partial<MeetingsSnapshot>) {
    if (!this.alive) return;
    this.snapshot = { ...this.snapshot, ...next };
    this.changed(this.snapshot);
  }

  async load() {
    await Promise.all([
      this.loadDevices(),
      this.refreshList(),
      this.loadCurrent(),
    ]);
  }

  async loadDevices() {
    if (!this.alive) return;
    const ticket = ++this.deviceTicket;
    this.update({ devicesLoading: true, devicesError: null });
    try {
      const devices = await this.api.devices();
      if (this.alive && ticket === this.deviceTicket)
        this.update({ devices, devicesLoading: false });
    } catch (error) {
      if (this.alive && ticket === this.deviceTicket)
        this.update({
          devicesLoading: false,
          devicesError: meetingErrorKey(error),
        });
    }
  }

  async refreshList() {
    if (!this.alive) return;
    const ticket = ++this.listTicket;
    const since = this.revision;
    this.update({ listLoading: true, listError: null });
    try {
      let meetings = await this.api.list();
      if (!this.alive || ticket !== this.listTicket) return;
      for (const [id, change] of this.changes) {
        if (change.revision <= since) continue;
        if (!change.summary) meetings = meetings.filter((row) => row.id !== id);
        else {
          const listed = meetings.find((row) => row.id === id);
          if (
            !listed ||
            summaryRevision(change.summary) >= summaryRevision(listed)
          )
            meetings = replaceSummary(meetings, change.summary);
        }
      }
      const previousSelected = this.snapshot.meetings.find(
        (row) => row.id === this.snapshot.selectedId,
      );
      meetings = meetings.map(newestSummary);
      let active = this.snapshot.active;
      for (const summary of meetings) {
        if (isProcessingTerminal(summary))
          cancellationRequests.delete(summary.id);
        active = activeAfterSummary(active, summary);
      }
      if (active) meetings = replaceSummary(meetings, active);
      meetings.sort((a, b) => b.created_at_ms - a.created_at_ms);
      this.update({
        meetings,
        active,
        listLoading: false,
        cancellingId:
          active && cancellationRequests.has(active.id) ? active.id : null,
      });
      const selectedId = this.snapshot.selectedId;
      if (selectedId && !meetings.some((row) => row.id === selectedId)) {
        ++this.transcriptTicket;
        this.update({
          selectedId: null,
          transcript: null,
          transcriptLoading: false,
        });
      } else if (selectedId) {
        const selected = meetings.find((row) => row.id === selectedId);
        if (
          selected &&
          previousSelected?.status !== selected.status &&
          !isMeetingActive(selected)
        )
          void this.select(selectedId);
      }
    } catch (error) {
      if (this.alive && ticket === this.listTicket)
        this.update({ listLoading: false, listError: meetingErrorKey(error) });
    }
  }

  async loadCurrent() {
    if (!this.alive) return;
    const ticket = ++this.currentTicket;
    const since = this.revision;
    const activityAtRead = activityRevision;
    try {
      const active = await this.api.current();
      if (!this.alive || ticket !== this.currentTicket) return;
      // Native versions determine order. Legacy/null reads retain the local
      // in-flight guard used by older fixtures.
      if (active && (summaryRevision(active) > 0 || since === this.revision))
        this.receive(active);
      else if (
        !active &&
        since === this.revision &&
        activityAtRead === activityRevision
      ) {
        if (latestActivity) cancellationRequests.delete(latestActivity.id);
        latestActivity = null;
        this.update({ active: null, cancellingId: null });
      }
      this.update({ currentLoading: false, currentError: null });
    } catch (error) {
      if (this.alive && ticket === this.currentTicket)
        this.update({
          currentLoading: false,
          currentError: meetingErrorKey(error),
        });
    }
  }

  async pollCurrent() {
    if (
      !this.alive ||
      this.polling ||
      !this.snapshot.active ||
      this.snapshot.busy
    )
      return;
    this.polling = true;
    const previous = this.snapshot.active;
    await this.loadCurrent();
    if (this.alive && previous && !this.snapshot.active) {
      await this.refreshList();
      if (this.snapshot.selectedId === previous.id)
        await this.select(previous.id);
    }
    this.polling = false;
  }

  receive(summary: MeetingSummary) {
    if (!this.alive || isSummaryStale(summary)) return false;
    const previous = this.snapshot.meetings.find(
      (row) => row.id === summary.id,
    );
    latestSummaries.set(summary.id, summary);
    this.changes.set(summary.id, { revision: ++this.revision, summary });
    if (isProcessingTerminal(summary)) cancellationRequests.delete(summary.id);
    const active = activeAfterSummary(this.snapshot.active, summary);
    this.update({
      meetings: replaceSummary(this.snapshot.meetings, summary),
      active,
      cancellingId:
        active && cancellationRequests.has(active.id) ? active.id : null,
    });
    if (
      this.snapshot.selectedId === summary.id &&
      previous?.status !== summary.status &&
      !isMeetingActive(summary)
    )
      void this.select(summary.id);
    return true;
  }

  private commit(summary: MeetingSummary, since: number) {
    if (
      summaryRevision(summary) > 0 ||
      (this.changes.get(summary.id)?.revision ?? 0) <= since
    )
      return this.receive(summary);
    return false;
  }

  private cancellationId() {
    const id = this.snapshot.active?.id;
    return id && cancellationRequests.has(id) ? id : null;
  }

  async select(id: string) {
    if (!this.alive) return;
    const ticket = ++this.transcriptTicket;
    this.update({
      selectedId: id,
      transcript: null,
      transcriptLoading: true,
      transcriptError: null,
      exported: false,
    });
    try {
      const transcript = await this.api.transcript(id);
      if (!this.alive || ticket !== this.transcriptTicket) return;
      this.update({ transcript, transcriptLoading: false });
    } catch (error) {
      if (this.alive && ticket === this.transcriptTicket)
        this.update({
          transcriptLoading: false,
          transcriptError: meetingErrorKey(error),
        });
    }
  }

  private async mutate(kind: MeetingOperation, operation: () => Promise<void>) {
    if (!this.alive || this.snapshot.busy) return;
    this.update({ busy: kind, error: null, exported: false });
    try {
      await operation();
    } catch (error) {
      this.update({ error: meetingErrorKey(error) });
    } finally {
      this.update({ busy: null });
    }
  }

  start(options: MeetingStartOptions) {
    if (this.snapshot.active) return Promise.resolve();
    const microphone = this.snapshot.devices.some(
      (device) =>
        device.source === "microphone" && device.id === options.microphoneId,
    );
    const playback = this.snapshot.devices.some(
      (device) => device.source === "remote" && device.id === options.renderId,
    );
    if (!microphone || !playback) {
      this.update({ error: "devices" });
      return Promise.resolve();
    }
    return this.mutate("start", async () => {
      const since = this.revision;
      const selection = this.transcriptTicket;
      const summary = await this.api.start(options);
      if (!this.alive) return;
      const accepted = this.commit(summary, since);
      if (accepted && selection === this.transcriptTicket)
        await this.select(summary.id);
    });
  }

  finish(id: string) {
    if (
      this.snapshot.active?.id !== id ||
      !["recording", "paused"].includes(this.snapshot.active.status)
    )
      return Promise.resolve();
    return this.mutate("finish", async () => {
      const since = this.revision;
      const stopped = await this.api.stop(id);
      // Finishing is one user action. A page switch must not interrupt the
      // already requested transcription after native audio has been saved.
      this.commit(stopped, since);
      const beforeTranscription = this.revision;
      const processing = await this.api.transcribe(id);
      if (!this.alive) return;
      const accepted = this.commit(processing, beforeTranscription);
      if (accepted && this.snapshot.selectedId === id) await this.select(id);
    });
  }

  retry(id: string) {
    const meeting = this.snapshot.meetings.find((row) => row.id === id);
    if (this.snapshot.active || !meeting || !canTranscribeMeeting(meeting))
      return Promise.resolve();
    return this.mutate("transcribe", async () => {
      // A retry is a new job for the retained recording.
      cancellationRequests.delete(id);
      const since = this.revision;
      const summary = await this.api.transcribe(id);
      if (!this.alive) return;
      const accepted = this.commit(summary, since);
      if (accepted && this.snapshot.selectedId === id) await this.select(id);
    });
  }

  cancel(id: string) {
    if (
      this.snapshot.active?.id !== id ||
      this.snapshot.active.status !== "processing" ||
      cancellationRequests.has(id)
    )
      return Promise.resolve();
    return this.mutate("cancel", async () => {
      const since = this.revision;
      const request = Symbol(id);
      cancellationRequests.set(id, request);
      this.update({ cancellingId: id });
      try {
        const summary = await this.api.cancelTranscription(id);
        if (
          isProcessingTerminal(summary) &&
          !isSummaryStale(summary) &&
          cancellationRequests.get(id) === request
        )
          cancellationRequests.delete(id);
        if (!this.alive) return;
        const accepted = this.commit(summary, since);
        this.update({ cancellingId: this.cancellationId() });
        if (accepted && this.snapshot.selectedId === id) await this.select(id);
      } catch (error) {
        if (cancellationRequests.get(id) === request)
          cancellationRequests.delete(id);
        this.update({ cancellingId: this.cancellationId() });
        throw error;
      }
    });
  }

  remove(id: string) {
    const meeting = this.snapshot.meetings.find((row) => row.id === id);
    if (!meeting || isMeetingActive(meeting)) return Promise.resolve();
    return this.mutate("delete", async () => {
      await this.api.delete(id);
      if (!this.alive) return;
      this.changes.set(id, { revision: ++this.revision, summary: null });
      this.update({
        meetings: this.snapshot.meetings.filter((row) => row.id !== id),
      });
      if (this.snapshot.selectedId === id) {
        ++this.transcriptTicket;
        this.update({
          selectedId: null,
          transcript: null,
          transcriptLoading: false,
        });
      }
    });
  }

  export(
    id: string,
    format: MeetingExportFormat,
    language: MeetingExportLanguage,
    choose: () => Promise<string | null>,
  ) {
    const meeting = this.snapshot.meetings.find((row) => row.id === id);
    if (!meeting || isMeetingActive(meeting)) return Promise.resolve();
    return this.mutate("export", async () => {
      const selection = this.transcriptTicket;
      const destination = await choose();
      // Confirmation belongs to the original export action, even after a page switch.
      if (!destination) return;
      await this.api.export(id, destination, format, language);
      if (
        this.snapshot.selectedId === id &&
        selection === this.transcriptTicket
      )
        this.update({ exported: true });
    });
  }

  dispose() {
    this.alive = false;
    ++this.transcriptTicket;
    ++this.listTicket;
    ++this.currentTicket;
    ++this.deviceTicket;
  }
}
