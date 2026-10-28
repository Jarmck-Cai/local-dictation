import { invoke } from "@tauri-apps/api/core";
import type { HistoryEntry } from "../bindings";

export interface HistorySearchRequest {
  cursor: number | null;
  limit: number;
  query: string | null;
  anchor_id: number | null;
}

export interface HistorySearchPage {
  entries: HistoryEntry[];
  has_more: boolean;
  next_cursor: number | null;
  anchor_id: number | null;
}

export const searchHistoryEntries = (request: HistorySearchRequest) =>
  invoke<HistorySearchPage>("search_history_entries", {
    cursor: request.cursor,
    limit: request.limit,
    query: request.query,
    anchorId: request.anchor_id,
  });
