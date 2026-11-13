import { api } from './api';
import { subscribeEvents } from './sse';
import type { MintSnapshot, SavedMintEntry, Scene, Tab } from './types';

const BASE32_RE = /^[a-z2-7]+$/;
const SPARKLINE_POINTS = 60;

const emptySnapshot: MintSnapshot = {
  active: false,
  completed: false,
  target: '',
  best_address: null,
  best_len: 0,
  target_len: 0,
  total_hashes: 0,
  hashes_since_best: 0,
  hashes_per_sec: 0,
  elapsed_secs: 0,
  progress: 0,
  pace_ratio: 0,
  eta_secs: null
};

class OnyunStore {
  scene = $state<Scene>('loading');
  tab = $state<Tab>('mint');
  leftCollapsed = $state(false);
  leftWidth = $state(236);
  mobileNavOpen = $state(false);
  drawerOpen = $state(false);

  version = $state('0.1.0');
  engine = $state('CPU');
  cpuThreads = $state(4);
  outDir = $state('./mints');
  authRequired = $state(false);

  targetInput = $state('onyun');
  threadsInput = $state(4);

  snapshot = $state<MintSnapshot>({ ...emptySnapshot });
  rateHistory = $state<number[]>([]);
  mints = $state<SavedMintEntry[]>([]);
  selectedMintTarget = $state<string | null>(null);
  toastMessage = $state<string | null>(null);
  busy = $state(false);
  sseConnected = $state(false);

  private stopSse: (() => void) | null = null;
  private toastTimer: number | undefined = undefined;
  private lastBestLen = 0;

  constructor() {
    void this.init();
    if (typeof document !== 'undefined') {
      document.addEventListener('visibilitychange', () => {
        if (document.visibilityState === 'visible' && this.scene === 'workspace') {
          void this.syncOnResume();
        }
      });
    }
  }

  private async syncOnResume() {
    try {
      const status = await api.status();
      this.applySnapshot(status.snapshot);
      await this.refreshMints();
      if (!this.sseConnected) {
        this.connectSse();
      }
    } catch {
      // Ignore transient resume errors
    }
  }

  get normalizedTarget(): string {
    return this.targetInput.trim().toLowerCase();
  }

  get invalidChars(): string[] {
    const out: string[] = [];
    for (const ch of this.normalizedTarget) {
      if (!/[a-z2-7]/.test(ch) && !out.includes(ch)) {
        out.push(ch);
      }
    }
    return out;
  }

  get targetError(): string | null {
    const t = this.normalizedTarget;
    if (!t) return 'Enter a target prefix (a–z, 2–7).';
    if (t.length > 52) return `Max prefix length is 52 (${t.length} entered).`;
    if (this.invalidChars.length > 0) {
      return `Invalid Base32 char(s): ${this.invalidChars.map((c) => `'${c}'`).join(', ')} (only a–z, 2–7)`;
    }
    if (!BASE32_RE.test(t)) return 'Invalid Base32 prefix.';
    return null;
  }

  async init() {
    try {
      const status = await api.status();
      this.version = status.version;
      this.engine = status.engine;
      this.cpuThreads = status.cpu_threads;
      this.threadsInput = status.cpu_threads;
      this.outDir = status.out_dir;
      this.authRequired = status.auth_required;

      if (status.auth_required && !status.authenticated) {
        this.scene = 'login';
        return;
      }

      this.applySnapshot(status.snapshot);
      if (status.snapshot.target) {
        this.targetInput = status.snapshot.target;
      }
      this.scene = 'workspace';
      await this.refreshMints();
      this.connectSse();
    } catch {
      this.scene = 'workspace';
      this.toast('Failed to reach Onyun backend');
    }
  }

  private connectSse() {
    this.stopSse?.();
    this.stopSse = subscribeEvents(
      (snap) => {
        this.sseConnected = true;
        this.applySnapshot(snap);
      },
      () => {
        this.sseConnected = false;
      }
    );
  }

  private applySnapshot(snap: MintSnapshot) {
    const improved = snap.best_len > this.lastBestLen && snap.best_address;
    this.lastBestLen = snap.best_len;
    this.snapshot = snap;

    if (snap.active) {
      const mh = snap.hashes_per_sec / 1_000_000;
      const next = [...this.rateHistory, mh];
      if (next.length > SPARKLINE_POINTS) next.shift();
      this.rateHistory = next;
    }

    if (improved || snap.completed) {
      void this.refreshMints();
    }
  }

  async submitLogin(username: string, password: string) {
    await api.login(username, password);
    await this.init();
  }

  async logout() {
    this.stopSse?.();
    this.stopSse = null;
    await api.logout();
    this.scene = 'login';
  }

  async startMint(overrideTarget?: string) {
    const raw = overrideTarget ?? this.targetInput;
    const t = raw.trim().toLowerCase();
    this.targetInput = t;
    if (this.targetError) {
      this.toast(this.targetError);
      return;
    }
    this.busy = true;
    this.rateHistory = [];
    this.lastBestLen = 0;
    try {
      const snap = await api.startMint(t, this.threadsInput);
      this.applySnapshot(snap);
      this.tab = 'mint';
      await this.refreshMints();
    } catch (err) {
      this.toast(err instanceof Error ? err.message : 'Failed to start mint');
    } finally {
      this.busy = false;
    }
  }

  async stopMint() {
    this.busy = true;
    try {
      await api.stopMint();
      await this.refreshMints();
    } catch (err) {
      this.toast(err instanceof Error ? err.message : 'Failed to stop mint');
    } finally {
      this.busy = false;
    }
  }

  async refreshMints() {
    try {
      this.mints = await api.listMints();
    } catch {
      // Ignore transient errors
    }
  }

  async deleteMint(target: string) {
    try {
      await api.deleteMint(target);
      if (this.selectedMintTarget === target) {
        this.selectedMintTarget = null;
      }
      await this.refreshMints();
      this.toast(`Removed ./mints/${target}`);
    } catch (err) {
      this.toast(err instanceof Error ? err.message : 'Failed to delete mint');
    }
  }

  async copyText(value: string, label = 'Copied to clipboard') {
    try {
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(value);
        this.toast(label);
        return;
      }
    } catch {
      // Fall through to legacy copy
    }
    try {
      const ta = document.createElement('textarea');
      ta.value = value;
      ta.setAttribute('readonly', '');
      ta.style.position = 'fixed';
      ta.style.opacity = '0';
      document.body.appendChild(ta);
      ta.select();
      const ok = document.execCommand('copy');
      document.body.removeChild(ta);
      this.toast(ok ? label : 'Clipboard unavailable');
    } catch {
      this.toast('Clipboard unavailable');
    }
  }

  toast(message: string) {
    this.toastMessage = message;
    clearTimeout(this.toastTimer);
    this.toastTimer = window.setTimeout(() => {
      this.toastMessage = null;
    }, 2800);
  }
}

export const store = new OnyunStore();

export function formatHashrate(hps: number): string {
  if (!Number.isFinite(hps) || hps <= 0) return '0.00 MH/s';
  if (hps >= 1_000_000_000) return `${(hps / 1_000_000_000).toFixed(2)} GH/s`;
  if (hps >= 1_000_000) return `${(hps / 1_000_000).toFixed(2)} MH/s`;
  if (hps >= 1_000) return `${(hps / 1_000).toFixed(1)} kH/s`;
  return `${Math.round(hps)} H/s`;
}

export function formatCount(n: number): string {
  if (n >= 1_000_000_000_000) return `${(n / 1_000_000_000_000).toFixed(2)}T`;
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(2)}B`;
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

export function formatDuration(secs: number | null | undefined): string {
  if (secs === null || secs === undefined || !Number.isFinite(secs)) return '—';
  if (secs < 1) return '<1s';
  if (secs < 60) return `${Math.round(secs)}s`;
  if (secs < 3600) {
    const m = Math.floor(secs / 60);
    const s = Math.round(secs % 60);
    return s > 0 ? `${m}m ${s}s` : `${m}m`;
  }
  if (secs < 86400) return `${(secs / 3600).toFixed(1)}h`;
  if (secs < 86400 * 365) return `${(secs / 86400).toFixed(1)}d`;
  return '>1y';
}

export function formatRelativeTime(unixSecs: number): string {
  if (!unixSecs) return '—';
  const diff = Math.max(0, Math.floor(Date.now() / 1000) - unixSecs);
  if (diff < 60) return 'just now';
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}
