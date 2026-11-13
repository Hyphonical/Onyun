<script lang="ts">
  import { Cpu, LogOut } from '@lucide/svelte';
  import { store } from './lib/store.svelte';
  import MintView from './components/MintView.svelte';
  import MintsView from './components/MintsView.svelte';
  import LoginView from './components/LoginView.svelte';

  const snap = $derived(store.snapshot);
</script>

<div class="root-frame">
  {#if store.scene === 'loading'}
    <div class="boot-stage">
      <img src="/branding/icon.svg" alt="Onyun" class="boot-logo" />
      <span class="boot-label">Loading Onyun…</span>
    </div>
  {:else if store.scene === 'login'}
    <LoginView />
  {:else}
    <header class="bar">
      <img src="/branding/icon.svg" alt="" class="logo" />
      <span class="word">Onyun</span>
      <span class="ver mono">v{store.version}</span>
      <span class="grow"></span>
      <span class="engine mono" class:live={snap.active}>
        <span class="dot"></span>
        <Cpu size={13} />
        {store.engine} · {store.threadsInput}/{store.cpuThreads}
      </span>
      {#if store.authRequired}
        <button class="btn-icon" onclick={() => store.logout()} aria-label="Log out" title="Log out">
          <LogOut size={15} />
        </button>
      {/if}
    </header>
    <main class="stage"><MintView /></main>
    <MintsView />
  {/if}

  {#if store.toastMessage}
    <div class="toast" role="status">{store.toastMessage}</div>
  {/if}
</div>

<style>
  .root-frame {
    height: 100%;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .boot-stage {
    flex: 1;
    display: grid;
    place-content: center;
    justify-items: center;
    gap: 12px;
    color: var(--dim);
    font-size: 12.5px;
  }
  .boot-logo { width: 34px; height: 34px; border-radius: 8px; }
  .bar {
    height: 56px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 24px;
    border-bottom: 1px solid var(--border-subtle);
  }
  .logo { width: 28px; height: 28px; border-radius: 7px; }
  .word {
    font-family: var(--font-display);
    font-size: 26px;
    line-height: 1;
    color: var(--head);
    letter-spacing: 0.01em;
  }
  .ver { font-size: 11px; color: var(--muted); }
  .grow { flex: 1; }
  .engine {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    font-size: 11.5px;
    color: var(--dim);
    padding: 5px 11px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface-0);
  }
  .dot { width: 6px; height: 6px; border-radius: 50%; background: var(--muted); }
  .engine.live { color: var(--accent); border-color: rgba(194, 168, 108, 0.38); }
  .engine.live .dot {
    background: var(--accent);
    box-shadow: 0 0 8px rgba(194, 168, 108, 0.8);
    animation: pulse 1.8s ease-in-out infinite;
  }
  @keyframes pulse { 50% { opacity: 0.35; } }
  .stage {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 56px 24px 96px;
  }
  .toast {
    position: fixed;
    right: 20px;
    bottom: 64px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    color: var(--head);
    padding: 9px 14px;
    border-radius: var(--r-sm);
    font-size: 12.5px;
    z-index: 60;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.45);
  }
  @media (max-width: 820px) {
    .bar { padding: 0 14px; gap: 8px; }
    .stage { padding: 28px 14px 96px; }
  }
  @media (max-width: 520px) {
    .ver { display: none; }
    .word { font-size: 22px; }
    .engine { padding: 4px 9px; font-size: 11px; }
  }
</style>
