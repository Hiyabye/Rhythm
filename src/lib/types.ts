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
  state: 'ready' | 'unavailable' | 'failed' | 'loading' | 'busy';
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
export interface CatalogEntry {
  id: string; name: string; parameters: string; quantization: string;
  bytes: number; memory_bytes: number; description: string; use_case: string;
  repository: string; revision: string; filename: string; sha256: string; license: string;
  compatibility: 'verified' | 'experimental' | 'unsupported'; reason: string | null;
}
export interface AiModel {
  id: string; catalog_id: string | null; name: string; ownership: 'managed' | 'external'; path: string;
  sha256: string | null; expected_sha256: string | null; expected_bytes: number | null; bytes: number;
  source_revision: string | null; source_filename: string | null; status: string;
  downloaded_bytes: number; resume_etag: string | null; error: string | null; created_at: string; updated_at: string;
}
export interface AiPreferences {
  active_model_id: string | null; setup_completed: boolean; processing_mode: 'explicit' | 'recent';
  automatic_limit: number; max_tokens: number;
}
export interface AnalysisKey {
  document_id: number; revision_id: number; task: string; model_sha256: string;
  prompt_version: number; schema_version: number; configuration_hash: string;
}
export interface AiJob extends AnalysisKey {
  id: number; model_identity: { name?: string }; priority: number; status: string; error: string | null;
  duration_ms: number | null; created_at: string; updated_at: string;
}
export interface AnalysisFact {
  kind: string; label: string; value: string; evidence: string; date: string | null;
  uncertainty: 'clear' | 'ambiguous' | 'unknown';
}
export interface Analysis {
  short_summary: string; summary: string; category: string;
  uncertainty: 'clear' | 'ambiguous' | 'unknown'; facts: AnalysisFact[];
}
export interface AiResult extends AnalysisKey {
  id: number; job_id: number; model_identity: { name?: string; sha256?: string; max_tokens?: number }; result: Analysis;
  partial: boolean; processed_chars: number; total_chars: number; duration_ms: number; created_at: string;
}
export interface AiSnapshot {
  catalog: CatalogEntry[]; models: AiModel[]; preferences: AiPreferences; jobs: AiJob[]; runtime: WorkerStatus;
  hardware: { os: string; arch: string; ram_bytes: number; disk_available_bytes: number; backend: string };
  busy: boolean;
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
