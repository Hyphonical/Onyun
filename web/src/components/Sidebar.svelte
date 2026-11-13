<script lang="ts">
  import {
    Cpu,
    FolderKey,
    PanelLeftClose,
    PanelLeftOpen,
    LogOut,
    X,
    Sparkles
  } from '@lucide/svelte';
  import { store } from '../lib/store.svelte';
  import type { Tab } from '../lib/types';

  let { mobile = false }: { mobile?: boolean } = $props();

  const collapsed = $derived(!mobile && store.leftCollapsed);

  const navItems = $derived<{ id: Tab; label: string; count: string; icon: typeof Sparkles }[]>([
    {
      id: 'mint',
      label: 'Minter',
      count: store.snapshot.active ? 'LIVE' : 'IDLE',
      icon: Sparkles
    },
    {
      id: 'mints',
      label: 'Saved Mints',
      count: String(store.mints.length),
      icon: FolderKey
    }
  ]);

  function selectTab(id: Tab) {
    store.tab = id;
    if (mobile) store.mobileNavOpen = false;
  }
</script>

<aside class="sidebar" class:collapsed class:mobile>
  <div class="brand">
    <div class="brand-lockup">
      <img src="/branding/icon.svg" alt="Onyun" class="brand-icon" />
      {#if !collapsed}
        <span class="brand-word">Onyun</span>
      {/if}
    </div>

    {#if mobile}
      <button
        class="btn-icon"
        onclick={() => (store.mobileNavOpen = false)}
        aria-label="Close navigation"
      >
        <X size={16} />
      </button>
    {:else if !collapsed}
      <div class="brand-right">
        <span class="version-badge">v{store.version}</span>
        <button
          class="btn-icon"
          title="Collapse sidebar"
          onclick={() => (store.leftCollapsed = true)}
        >
          <PanelLeftClose size={15} />
        </button>
      </div>
    {/if}
  </div>

  {#if collapsed}
    <button
      class="btn-icon expand-rail-btn"
      title="Expand sidebar"
      onclick={() => (store.leftCollapsed = false)}
    >
      <PanelLeftOpen size={15} />
    </button>
  {/if}

  <nav class="nav" aria-label="Primary navigation">
    {#each navItems as item (item.id)}
      {@const Icon = item.icon}
      <button
        class="nav-item"
        class:active={store.tab === item.id}
        onclick={() => selectTab(item.id)}
        title={collapsed ? item.label : undefined}
      >
        <Icon size={15} class="nav-icon" />
        {#if !collapsed}
          <span class="nav-label">{item.label}</span>
          <span class="nav-count mono" class:live={item.id === 'mint' && store.snapshot.active}>
            {item.count}
          </span>
        {/if}
      </button>
    {/each}
  </nav>

  <div class="sidebar-foot">
    {#if !collapsed}
      <div class="engine-box">
        <div class="engine-row">
          <span class="engine-label"><Cpu size={12} /> Engine</span>
          <span class="mono engine-val">{store.engine}</span>
        </div>
        <div class="engine-row">
          <span class="engine-label">Cores</span>
          <span class="mono engine-val">{store.threadsInput}/{store.cpuThreads}</span>
        </div>
        <div class="engine-row">
          <span class="engine-label">Output</span>
          <span class="mono engine-val truncate" title={store.outDir}>{store.outDir}</span>
        </div>
      </div>
    {/if}

    {#if store.authRequired}
      <button
        class="nav-item logout-btn"
        onclick={() => store.logout()}
        title={collapsed ? 'Sign out' : undefined}
      >
        <LogOut size={15} />
        {#if !collapsed}
          <span class="nav-label">Sign out</span>
        {/if}
      </button>
    {/if}
  </div>
</aside>

<style>
  .sidebar {
    height: 100%;
    background: var(--surface-0);
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    padding: 14px 12px;
    gap: 14px;
    user-select: none;
  }

  .sidebar.collapsed {
    align-items: center;
    padding: 14px 8px;
  }

  .brand {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    min-height: 32px;
  }

  .brand-lockup {
    display: flex;
    align-items: center;
    gap: 9px;
    min-width: 0;
  }

  .brand-icon {
    width: 26px;
    height: 26px;
    border-radius: 7px;
    flex-shrink: 0;
  }

  .brand-word {
    font-family: var(--font-display);
    font-size: 20px;
    color: var(--head);
    letter-spacing: 0.01em;
    line-height: 1;
  }

  .brand-right {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .version-badge {
    font-family: var(--font-mono);
    font-size: 10.5px;
    color: var(--dim);
    background: var(--surface-2);
    border: 1px solid var(--border);
    padding: 1px 6px;
    border-radius: 999px;
  }

  .expand-rail-btn {
    width: 34px;
    height: 34px;
  }

  .nav {
    display: flex;
    flex-direction: column;
    gap: 4px;
    width: 100%;
  }

  .nav-item {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 8px 10px;
    border-radius: var(--r-sm);
    color: var(--dim);
    font-size: 13px;
    font-weight: 500;
    border: 1px solid transparent;
    transition: background 110ms, color 110ms, border-color 110ms;
  }

  .sidebar.collapsed .nav-item {
    justify-content: center;
    padding: 9px;
  }

  .nav-item:hover {
    background: var(--surface-2);
    color: var(--fg);
  }

  .nav-item.active {
    background: var(--surface-2);
    border-color: var(--border);
    color: var(--head);
  }

  .nav-label {
    flex: 1;
    text-align: left;
  }

  .nav-count {
    font-size: 10.5px;
    color: var(--dim);
    background: var(--surface-1);
    border: 1px solid var(--border-subtle);
    padding: 1px 6px;
    border-radius: 999px;
  }

  .nav-count.live {
    color: var(--accent);
    border-color: rgba(194, 168, 108, 0.4);
    background: var(--accent-soft);
  }

  .sidebar-foot {
    margin-top: auto;
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: 100%;
  }

  .engine-box {
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--r-sm);
    padding: 10px 11px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .engine-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    font-size: 11.5px;
  }

  .engine-label {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    color: var(--dim);
  }

  .engine-val {
    color: var(--fg);
    font-size: 11px;
    max-width: 120px;
  }

  .truncate {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
