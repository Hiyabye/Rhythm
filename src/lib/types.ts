export type Theme = 'system' | 'light' | 'dark';
export type EnergyProfile = 'eco' | 'balanced' | 'performance';

export interface Settings {
  theme: Theme;
  energy_profile: EnergyProfile;
  interests: string;
}
export interface Extension {
  id: string;
  display_name: string;
  description: string;
  available: boolean;
  enabled: boolean;
}
export interface WorkerStatus {
  state: 'ready' | 'unavailable' | 'failed';
  runtime_available: boolean;
  model: string | null;
  detail: string | null;
}
export interface Snapshot {
  settings: Settings;
  extensions: Extension[];
  storage: { path: string; bytes: number; schema_version: number };
  worker: WorkerStatus;
}
export interface SourceInfo {
  id: string;
  name: string;
  organization: string;
  description: string;
  url: string;
  family: string;
  supported: boolean;
  subscribed: boolean;
  last_attempt: string | null;
  last_success: string | null;
  last_error: string | null;
  next_page: number;
  history_complete: boolean;
  status: string;
  discovered: number;
  archived: number;
  failed: number;
}
export interface DocumentItem {
  id: number;
  source_id: string;
  source_name: string;
  title: string;
  author: string | null;
  category: string | null;
  published_at: string;
  collected_at: string;
  is_read: boolean;
  saved: boolean;
  historical: boolean;
  snippet: string;
  content_status: string;
}
export interface ResourceRecord {
  id: number;
  url: string;
  filename: string;
  kind: 'image' | 'attachment';
  status: string;
  mime: string | null;
  bytes: number | null;
  local_path: string | null;
  sha256: string | null;
  error: string | null;
}
export interface RevisionInfo {
  id: number;
  collected_at: string;
  content_hash: string;
}
export interface DocumentDetail {
  item: DocumentItem;
  canonical_url: string;
  body_html: string;
  body_text: string;
  revision_id: number;
  revisions: RevisionInfo[];
  resources: ResourceRecord[];
}
export interface ArchiveQuery {
  query: string;
  source_id: string | null;
  category: string | null;
  saved_only: boolean;
  unread_only: boolean;
  ascending: boolean;
  offset: number;
  limit: number | null;
}
export interface ArchivePage {
  items: DocumentItem[];
  total: number;
  categories: string[];
}
export interface ArchiveSummary {
  documents: number;
  unread: number;
  saved: number;
  new_since_sync: number;
  resource_bytes: number;
}
export interface SyncStatus {
  running: boolean;
  source_id: string | null;
  mode: string;
  page: number;
  message: string;
}
