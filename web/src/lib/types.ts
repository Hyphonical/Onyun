export interface MintSnapshot {
  active: boolean;
  completed: boolean;
  target: string;
  best_address: string | null;
  best_len: number;
  target_len: number;
  total_hashes: number;
  hashes_since_best: number;
  hashes_per_sec: number;
  elapsed_secs: number;
  progress: number;
  pace_ratio: number;
  eta_secs: number | null;
}

export interface StatusPayload {
  version: string;
  engine: string;
  device_name: string;
  cpu_threads: number;
  out_dir: string;
  auth_required: boolean;
  authenticated: boolean;
  snapshot: MintSnapshot;
}

export interface SavedMintEntry {
  target: string;
  best_address: string;
  matched_len: number;
  target_len: number;
  completed: boolean;
  modified_at: number;
  path: string;
}

export type Scene = 'loading' | 'login' | 'workspace';
export type Tab = 'mint' | 'mints';
