import { invoke } from "@tauri-apps/api/core";

export type CollectedTermsFilter = "all" | "liked" | "deleted";
export interface CollectedTerm {
  id: number;
  term: string;
  record_count: number;
  last_seen: number | null;
  liked: boolean;
  deleted: boolean;
}
export interface CollectedTermsPage {
  entries: CollectedTerm[];
  total: number;
  offset: number;
  limit: number;
}
export interface CollectedTermsQuery {
  query: string;
  filter: CollectedTermsFilter;
  offset: number;
  limit: number;
}
export interface CollectedTermsTransport {
  getPage: (query: CollectedTermsQuery) => Promise<CollectedTermsPage>;
  setLiked: (id: number, liked: boolean) => Promise<void>;
  setDeleted: (id: number, deleted: boolean) => Promise<void>;
}
export const collectedTermsApi: CollectedTermsTransport = {
  getPage: (query) => invoke("get_collected_terms", { ...query }),
  setLiked: (id, liked) => invoke("set_collected_term_liked", { id, liked }),
  setDeleted: (id, deleted) =>
    invoke("set_collected_term_deleted", { id, deleted }),
};
export const COLLECTED_TERMS_UPDATED = "collected-terms-updated";
