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
  storage: {
    path: string;
    bytes: number;
    schema_version: number;
  };
  worker: WorkerStatus;
}
