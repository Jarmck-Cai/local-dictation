import { invoke } from "@tauri-apps/api/core";

// These small DTOs mirror meetings/contracts.rs. Audio and storage paths stay native.
export type MeetingSource = "microphone" | "remote";
export type MeetingStatus =
  | "recording"
  | "paused"
  | "recorded"
  | "interrupted"
  | "processing"
  | "complete"
  | "cancelled"
  | "failed";

export interface MeetingDevice {
  id: string;
  name: string;
  source: MeetingSource;
  is_default: boolean;
}

export interface MeetingSummary {
  id: string;
  /** Increasing within one native app run; older fixtures may omit it. */
  revision?: number;
  title: string;
  created_at_ms: number;
  duration_ms: number;
  status: MeetingStatus;
  error: string | null;
  capture_error?: string | null;
  progress_percent: number | null;
  microphone_name: string | null;
  playback_name: string | null;
}

export interface MeetingSegment {
  id: number;
  source: MeetingSource;
  start_ms: number;
  end_ms: number;
  text: string;
}

export interface MeetingTranscript {
  id: string;
  complete: boolean;
  segments: MeetingSegment[];
  warnings: string[];
}

export type MeetingExportFormat = "audio" | "text";
export type MeetingExportLanguage = "zh" | "en";

export interface MeetingStartOptions {
  title: string;
  microphoneId: string;
  renderId: string;
}

export interface MeetingsTransport {
  devices(): Promise<MeetingDevice[]>;
  list(): Promise<MeetingSummary[]>;
  current(): Promise<MeetingSummary | null>;
  start(options: MeetingStartOptions): Promise<MeetingSummary>;
  stop(id: string): Promise<MeetingSummary>;
  transcribe(id: string): Promise<MeetingSummary>;
  cancelTranscription(id: string): Promise<MeetingSummary>;
  transcript(id: string): Promise<MeetingTranscript | null>;
  delete(id: string): Promise<void>;
  export(
    id: string,
    destination: string,
    format: MeetingExportFormat,
    language: MeetingExportLanguage,
  ): Promise<void>;
}

export const meetingsApi: MeetingsTransport = {
  devices: () => invoke<MeetingDevice[]>("meeting_devices"),
  list: () => invoke<MeetingSummary[]>("meeting_list"),
  current: () => invoke<MeetingSummary | null>("meeting_current"),
  start: (options) => invoke<MeetingSummary>("meeting_start", { ...options }),
  stop: (id) => invoke<MeetingSummary>("meeting_stop", { id }),
  transcribe: (id) => invoke<MeetingSummary>("meeting_transcribe", { id }),
  cancelTranscription: (id) =>
    invoke<MeetingSummary>("meeting_cancel_transcription", { id }),
  transcript: (id) =>
    invoke<MeetingTranscript | null>("meeting_transcript", { id }),
  delete: (id) => invoke<void>("meeting_delete", { id }),
  export: (id, destination, format, language) =>
    invoke<void>("meeting_export", { id, destination, format, language }),
};

export const MEETING_UPDATED_EVENT = "meeting-updated";
