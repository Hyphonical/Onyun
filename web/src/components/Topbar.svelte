<script lang="ts">
  import { Menu, Play, Square } from '@lucide/svelte';
  import { formatHashrate, store } from '../lib/store.svelte';

  const snap = $derived(store.snapshot);
</script>

<header class="topbar">
  <div class="topbar-left">
    <button
      class="btn-icon mobile-menu-btn"
      onclick={() => (store.mobileNavOpen = true)}
      aria-label="Open navigation"
    >
      <Menu size={18} />
    </button>

    <div class="status-pill">
      <span class="dot" class:active={snap.active} class:done={snap.completed}></span>
      <span class="mono status-text">
        {#if snap.active}
          {snap.target} · {formatHashrate(snap.hashes_per_sec)}
        {:else if snap.completed}
          {snap.target} · completed
        {:else}
          ready · {store.cpuThreads} threads available
        {/if}
      </span>
    </div>
  </div>

  <div class="topbar-actions">
    {#if snap.active}
      <button class="btn btn-danger btn-sm" disabled={store.busy} onclick={() => store.stopMint()}>
        <Square size={12} />
        <span>Stop</span>
      </button>
    {:else}
      <button
        class="btn btn-primary btn-sm"
        disabled={store.busy || Boolean(store.targetError)}
        onclick={() => store.startMint()}
      >
        <Play size={12} />
        <span>Start Minting</span>
      </button>
    {/if}
  </div>
</header>

<style>
  .topbar {
    height: 52px;
    flex-shrink: 0;
    border-bottom: 1px solid var(--border);
    background: var(--surface-0);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 0 20px;
  }

  .topbar-left {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  .mobile-menu-btn {
    display: none;
  }

  .status-pill {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 5px 12px;
    min-width: 0;
  }

  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--muted);
    flex-shrink: 0;
  }

  .dot.active {
    background: var(--accent);
    box-shadow: 0 0 8px rgba(194, 168, 108, 0.7);
  }

  .dot.done {
    background: var(--locked);
  }

  .status-text {
    font-size: 11.5px;
    color: var(--fg);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .topbar-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  @media (max-width: 820px) {
    .topbar {
      padding: 0 14px;
    }

    .mobile-menu-btn {
      display: inline-grid;
    }
  }
</style>
