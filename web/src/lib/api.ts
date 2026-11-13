import type { MintSnapshot, SavedMintEntry, StatusPayload } from './types';

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, {
    credentials: 'same-origin',
    headers: {
      'Content-Type': 'application/json',
      ...(init?.headers ?? {})
    },
    ...init
  });
  const text = await res.text();
  let data: unknown = null;
  if (text) {
    try {
      data = JSON.parse(text);
    } catch {
      data = text;
    }
  }
  if (!res.ok) {
    const msg =
      typeof data === 'object' && data !== null && 'error' in data
        ? String(data.error)
        : `Request failed (${res.status})`;
    throw new Error(msg);
  }
  return data as T;
}

export const api = {
  status: () => request<StatusPayload>('/api/status'),
  login: (username: string, password: string) =>
    request<{ ok: boolean }>('/api/auth/login', {
      method: 'POST',
      body: JSON.stringify({ username, password })
    }),
  logout: () =>
    request<{ ok: boolean }>('/api/auth/logout', {
      method: 'POST'
    }),
  startMint: (target: string, threads?: number) =>
    request<MintSnapshot>('/api/mint/start', {
      method: 'POST',
      body: JSON.stringify({ target, threads })
    }),
  stopMint: () =>
    request<{ ok: boolean }>('/api/mint/stop', {
      method: 'POST'
    }),
  listMints: () => request<SavedMintEntry[]>('/api/mints'),
  deleteMint: (target: string) =>
    request<{ ok: boolean }>(`/api/mints/${encodeURIComponent(target)}`, {
      method: 'DELETE'
    })
};
