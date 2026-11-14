import type { MintSnapshot } from './types';

export function subscribeEvents(
  onSnapshot: (snap: MintSnapshot) => void,
  onError?: () => void
): () => void {
  const es = new EventSource('/api/events', { withCredentials: true });
  es.addEventListener('snapshot', (ev) => {
    try {
      const snap = JSON.parse((ev as MessageEvent<string>).data) as MintSnapshot;
      onSnapshot(snap);
    } catch {
    }
  });
  es.onerror = () => {
    onError?.();
  };
  return () => {
    es.close();
  };
}
