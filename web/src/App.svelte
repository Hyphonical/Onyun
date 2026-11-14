<script lang="ts">
  import { Check, ChevronDown, Cpu, Github, LogOut } from '@lucide/svelte';
  import { store } from './lib/store.svelte';
  import MintView from './components/MintView.svelte';
  import MintsView from './components/MintsView.svelte';
  import LoginView from './components/LoginView.svelte';

  const snap = $derived(store.snapshot);
  let devicesOpen = $state(false);

  function handleWindowClick(e: MouseEvent) {
    if (!devicesOpen) return;
    const target = e.target as HTMLElement | null;
    if (!target?.closest('.device-picker')) {
      devicesOpen = false;
    }
  }
</script>

<svelte:window onclick={handleWindowClick} />

<div class="root-frame">
  {#if store.scene === 'loading'}
    <div class="boot-stage">
      <span class="boot-label">Loading Onyun…</span>
    </div>
  {:else if store.scene === 'login'}
    <LoginView />
  {:else}
    <div class="corner-brand">
      <span class="pill">
        <svg class="rings" viewBox="0 0 24 24" fill="none" aria-hidden="true">
          <circle cx="12" cy="12" r="9" stroke="#c2a86c" stroke-width="2.2" />
          <circle cx="12" cy="12" r="5.6" stroke="#ffffd4" stroke-width="1.9" />
          <circle cx="12" cy="12" r="2.4" stroke="#afb97a" stroke-width="1.6" />
        </svg>
        <span class="mono ver">v{store.version}</span>
      </span>
    </div>
    <div class="top-cluster">
      <div class="device-picker">
        <button
          type="button"
          class="engine mono"
          class:live={snap.active}
          class:open={devicesOpen}
          title={store.activeDeviceSummary}
          aria-expanded={devicesOpen}
          onclick={() => (devicesOpen = !devicesOpen)}
        >
          <span class="dot"></span>
          <Cpu size={13} />
          <span class="device-name">{store.activeDeviceSummary}</span>
          {#if store.usesCpu}
            <span class="sep">·</span>
            <span>{store.threadsInput}/{store.cpuThreads}</span>
          {/if}
          <ChevronDown size={13} class="caret" />
        </button>
        {#if devicesOpen}
          <div class="device-menu" role="dialog" aria-label="Compute devices">
            <div class="device-menu-head">
              <span>Compute devices</span>
              <span class="mono hint">{store.engineMode}</span>
            </div>
            {#each store.devices as dev (dev.id)}
              {@const checked = store.enabledDevices.includes(dev.id)}
              <button
                type="button"
                class="device-item"
                class:checked
                disabled={snap.active}
                onclick={() => store.toggleDevice(dev.id)}
              >
                <span class="chk" class:on={checked} aria-hidden="true">
                  {#if checked}<Check size={11} strokeWidth={3} />{/if}
                </span>
                <span class="dev-kind mono">{dev.kind.toUpperCase()}</span>
                <span class="dev-label">{dev.name}</span>
              </button>
            {/each}
          </div>
        {/if}
      </div>
      <a
        class="gh-link"
        href="https://github.com/Hyphonical/Onyun"
        target="_blank"
        rel="noopener noreferrer"
        aria-label="GitHub repository"
        title="Hyphonical/Onyun on GitHub"
      >
        <Github size={15} />
      </a>
      {#if store.authRequired}
        <button class="btn-icon" onclick={() => store.logout()} aria-label="Log out" title="Log out">
          <LogOut size={15} />
        </button>
      {/if}
    </div>
    <main class="stage"><MintView /></main>
    <MintsView />
  {/if}

  {#if store.toastMessage}
    <div class="toast" role="status">{store.toastMessage}</div>
  {/if}
</div>

<style>
  .root-frame {
    position: relative;
    height: 100%;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .corner-brand {
    position: fixed;
    top: 16px;
    left: 22px;
    z-index: 20;
  }
  .pill {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 5px 12px 5px 9px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface-0);
    box-shadow: var(--shadow-raised);
  }
  .rings { width: 22px; height: 22px; flex-shrink: 0; }
  .ver { font-size: 11.5px; color: var(--dim); }
  .boot-stage {
    flex: 1;
    display: grid;
    place-content: center;
    color: var(--dim);
    font-size: 12.5px;
  }
  .top-cluster {
    position: fixed;
    top: 20px;
    right: 26px;
    z-index: 20;
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .gh-link {
    display: inline-grid;
    place-items: center;
    height: 32px;
    padding: 0 12px;
    text-decoration: none;
    color: var(--dim);
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface-0);
    box-shadow: var(--shadow-raised);
  }
  .device-name {
    max-width: 230px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sep { color: var(--muted); }
  .engine {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 13.5px;
    color: var(--dim);
    padding: 6px 14px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface-0);
    box-shadow: var(--shadow-raised);
  }
  .dot { width: 6px; height: 6px; border-radius: 50%; background: var(--muted); }
  .engine.live { color: var(--accent); border-color: rgba(194, 168, 108, 0.38); }
  .engine.live .dot {
    background: var(--accent);
    box-shadow: 0 0 8px rgba(194, 168, 108, 0.8);
    animation: pulse 1.8s ease-in-out infinite;
  }
  @keyframes pulse { 50% { opacity: 0.35; } }
  .device-picker { position: relative; }
  .engine { cursor: pointer; transition: border-color 140ms, color 140ms; }
  .engine:hover, .engine.open { border-color: rgba(194, 168, 108, 0.45); color: var(--fg); }
  .engine :global(.caret) { color: var(--muted); transition: transform 160ms; }
  .engine.open :global(.caret) { transform: rotate(180deg); }
  .device-menu {
    position: absolute;
    top: calc(100% + 8px);
    right: 0;
    min-width: 280px;
    max-width: 340px;
    padding: 8px;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
    box-shadow: 0 14px 36px rgba(0, 0, 0, 0.52);
    display: flex;
    flex-direction: column;
    gap: 4px;
    z-index: 40;
  }
  .device-menu-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 4px 8px 6px;
    font-size: 11px;
    color: var(--dim);
    border-bottom: 1px solid var(--border-subtle);
    margin-bottom: 2px;
  }
  .device-menu-head .hint { color: var(--accent); text-transform: uppercase; }
  .device-item {
    display: flex;
    align-items: center;
    gap: 9px;
    width: 100%;
    padding: 7px 8px;
    border-radius: var(--r-sm);
    color: var(--dim);
    text-align: left;
    font-size: 12.5px;
    transition: background 110ms, color 110ms;
  }
  .device-item:hover:not(:disabled) { background: var(--surface-2); color: var(--fg); }
  .device-item.checked { color: var(--head); }
  .device-item:disabled { opacity: 0.55; cursor: not-allowed; }
  .chk {
    width: 16px;
    height: 16px;
    flex-shrink: 0;
    border-radius: 4px;
    border: 1px solid var(--border);
    background: var(--bg);
    display: grid;
    place-items: center;
    color: var(--bg);
    transition: background 120ms, border-color 120ms, box-shadow 120ms;
  }
  .chk.on {
    background: var(--accent);
    border-color: var(--accent);
    box-shadow: 0 0 10px rgba(194, 168, 108, 0.35);
  }
  .dev-kind {
    font-size: 10px;
    padding: 1px 5px;
    border-radius: 4px;
    background: var(--surface-2);
    color: var(--accent);
    border: 1px solid var(--border-subtle);
    flex-shrink: 0;
  }
  .dev-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .stage {
    position: relative;
    z-index: 1;
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    justify-content: center;
    padding: 64px 28px 96px;
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
    .top-cluster { top: 12px; right: 14px; gap: 6px; }
    .stage { padding: 56px 14px 96px; }
  }
  @media (max-width: 520px) {
    .device-name { max-width: 110px; }
    .engine { padding: 4px 9px; font-size: 11px; }
  }
  @media (prefers-reduced-motion: reduce) {
    .engine.live .dot { animation: none; }
  }
</style>
