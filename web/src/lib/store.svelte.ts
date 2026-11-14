import { api } from './api';
import { subscribeEvents } from './sse';
import type { ComputeDevice, EngineMode, MintSnapshot, SavedMintEntry, Scene, Tab } from './types';

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

  version = $state('0.2.0');
  engine = $state('CPU');
  engineMode = $state<EngineMode>('auto');
  deviceName = $state('CPU');
  devices = $state<ComputeDevice[]>([]);
  enabledDevices = $state<string[]>([]);
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
      this.engineMode = status.engine_kind ?? 'auto';
      this.deviceName = status.device_name || status.engine;
      this.devices = status.devices ?? [];
      if (this.enabledDevices.length === 0 && this.devices.length > 0) {
        this.enabledDevices = this.devices.map((d) => d.id);
      }
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

  get hasGpu(): boolean {
    return this.devices.some((d) => d.kind === 'gpu');
  }

  get activeDeviceSummary(): string {
    if (this.devices.length === 0) return this.deviceName;
    if (this.engineMode === 'cpu') {
      return this.devices.find((d) => d.kind === 'cpu')?.name ?? 'CPU';
    }
    if (this.engineMode === 'gpu' || this.engineMode === 'auto') {
      const gpus = this.devices.filter((d) => d.kind === 'gpu' && this.enabledDevices.includes(d.id));
      if (gpus.length > 0) return gpus.map((d) => d.name).join(' + ');
      return this.devices.find((d) => d.kind === 'cpu')?.name ?? this.deviceName;
    }
    const active = this.devices.filter((d) => this.enabledDevices.includes(d.id));
    return active.length > 0 ? active.map((d) => d.name).join(' + ') : this.deviceName;
  }

  get usesCpu(): boolean {
    if (this.engineMode === 'cpu') return true;
    if (this.engineMode === 'hybrid') return this.enabledDevices.includes('cpu');
    return !this.hasGpu;
  }

  toggleDevice(id: string) {
    if (this.snapshot.active) return;
    if (this.enabledDevices.includes(id)) {
      if (this.enabledDevices.length <= 1) {
        this.toast('At least one compute device must stay enabled');
        return;
      }
      this.enabledDevices = this.enabledDevices.filter((x) => x !== id);
    } else {
      this.enabledDevices = [...this.enabledDevices, id];
    }
    if (this.engineMode !== 'hybrid') {
      this.engineMode = 'hybrid';
    }
  }

  setEngineMode(mode: EngineMode) {
    if (this.snapshot.active) return;
    this.engineMode = mode;
    if (mode === 'cpu') {
      this.enabledDevices = this.devices.filter((d) => d.kind === 'cpu').map((d) => d.id);
    } else if (mode === 'gpu') {
      const gpus = this.devices.filter((d) => d.kind === 'gpu').map((d) => d.id);
      if (gpus.length > 0) this.enabledDevices = gpus;
    } else if (mode === 'hybrid' && this.enabledDevices.length === 0) {
      this.enabledDevices = this.devices.map((d) => d.id);
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
      const deviceFilter =
        this.engineMode === 'hybrid' || this.engineMode === 'gpu' || this.engineMode === 'auto'
          ? this.enabledDevices
          : undefined;
      const snap = await api.startMint(t, this.threadsInput, this.engineMode, deviceFilter);
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

  exportMint(target?: string) {
    const t = target ?? (this.snapshot.target || this.mints[0]?.target);
    if (!t) {
      this.toast('No saved mint available to export');
      return;
    }
    const a = document.createElement('a');
    a.href = api.exportMintUrl(t);
    a.download = `${t}.tar`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
  }

  async copyText(value: string, label = 'Copied to clipboard') {
    try {
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(value);
        this.toast(label);
        return;
      }
    } catch {
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
