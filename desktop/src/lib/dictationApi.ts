import { invoke } from "@tauri-apps/api/core";

export interface CloudStatus {
  manual_paused: boolean;
  cooldown_remaining_ms: number;
  consecutive_failures: number;
}

export interface DictationTiming {
  id: string;
  history_id: number | null;
  recorded_at_ms: number;
  asr_ms: number;
  cleanup_ms: number;
  paste_ms: number | null;
  total_ms: number;
  cleanup_budget_ms: number;
  mode: string;
  outcome: string;
  cleanup_reason?: string | null;
  resource_snapshots?:
    | {
        stage: string;
        working_set_bytes: number | null;
        process_cpu_ms: number | null;
      }[]
    | null;
}

export interface VocabularyCandidate {
  id: number;
  term: string;
  replacement_from: string | null;
  context: string | null;
  source: string;
  occurrences: number;
}

export interface VocabularyTerm {
  id: number;
  term: string;
  aliases: string[];
  context: string | null;
  enabled: boolean;
}

export interface VocabularyCorrectionHit {
  term_id: number;
  term: string;
  from: string;
  count: number;
}

export interface VocabularyCorrectionSnapshot {
  recorded_at_ms: number | null;
  hits: VocabularyCorrectionHit[];
}

export interface VocabularyView {
  candidates: VocabularyCandidate[];
  entries: VocabularyTerm[];
}

export const dictationApi = {
  getCloudStatus: () => invoke<CloudStatus>("get_dictation_cloud_status"),
  setCloudPaused: (paused: boolean) =>
    invoke<CloudStatus>("set_dictation_cloud_paused", { paused }),
  toggleCloud: () => invoke<CloudStatus>("toggle_dictation_cloud"),
  getMetrics: () => invoke<DictationTiming[]>("get_dictation_metrics"),
  getVocabulary: () => invoke<VocabularyView>("get_vocabulary"),
  reviewCandidate: (id: number, accept: boolean) =>
    invoke<VocabularyView>("review_vocabulary_candidate", { id, accept }),
  deleteLearnedTerm: (id: number) =>
    invoke<VocabularyView>("delete_learned_term", { id }),
  updateLearnedTerm: (
    id: number,
    edit: { term: string; aliases: string[]; context: string | null },
  ) => invoke<VocabularyView>("update_learned_term", { id, ...edit }),
  setLearnedTermEnabled: (id: number, enabled: boolean) =>
    invoke<VocabularyView>("set_learned_term_enabled", { id, enabled }),
  getVocabularyCorrectionSnapshot: () =>
    invoke<VocabularyCorrectionSnapshot>("get_vocabulary_correction_snapshot"),
  submitCorrection: (historyId: number, correctedText: string) =>
    invoke<VocabularyView>("submit_correction_sample", {
      historyId,
      correctedText,
    }),
};

// Refresh sibling views after a local mutation without polling or clipboard access.
export const VOCABULARY_REFRESH = "dictation-vocabulary-refresh";
export const notifyVocabularyChanged = () =>
  window.dispatchEvent(new Event(VOCABULARY_REFRESH));
