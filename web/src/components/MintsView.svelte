<script lang="ts">
  import { Copy, Download, Play, Trash2, FolderKey, ChevronUp } from '@lucide/svelte';
  import { formatRelativeTime, store } from '../lib/store.svelte';
  const C = 2 * Math.PI * 9;
</script>

<section class="drawer" class:open={store.drawerOpen}>
  <button
    class="handle"
    aria-expanded={store.drawerOpen}
    onclick={() => (store.drawerOpen = !store.drawerOpen)}
  >
    <FolderKey size={15} />
    <span class="title">Saved mints</span>
    <span class="badge">{store.mints.length}</span>
    <span class="chev"><ChevronUp size={16} /></span>
  </button>

  <div class="list">
    {#if store.mints.length === 0}
      <p class="empty">Nothing saved yet. Mints save automatically whenever the match improves.</p>
    {:else}
      {#each store.mints as entry (entry.target)}
        {@const ratio = entry.target_len ? Math.min(1, entry.matched_len / entry.target_len) : 0}
        <div class="row">
          <svg class="ring" viewBox="0 0 24 24" aria-hidden="true">
            <circle cx="12" cy="12" r="9" class="trk" />
            <circle
              cx="12" cy="12" r="9"
              class="arc"
              class:done={entry.completed}
              stroke-dasharray="{C * ratio} {C}"
              transform="rotate(-90 12 12)"
            />
          </svg>

          <div class="meta">
            <span class="mono tgt">{entry.target}</span>
            <span class="mono cnt">{entry.matched_len}/{entry.target_len}{entry.completed ? ' · done' : ''}</span>
          </div>

          <span class="mono addr" title={entry.best_address}>
            <span class="matched">{entry.best_address.slice(0, entry.matched_len)}</span><span class="rest">{entry.best_address.slice(entry.matched_len)}</span>
          </span>

          <span class="age">{formatRelativeTime(entry.modified_at)}</span>

          <div class="acts">
            {#if !entry.completed}
              <button
                class="btn btn-sm"
                disabled={store.busy || store.snapshot.active}
                title={store.snapshot.active ? 'Stop the current mint first' : 'Continue minting this target'}
                onclick={() => store.startMint(entry.target)}
              >
                <Play size={12} /><span>Continue</span>
              </button>
            {/if}
            <button class="btn-icon" title="Copy .onion address" aria-label="Copy .onion address" onclick={() => store.copyText(entry.best_address, 'Copied .onion address')}>
              <Copy size={14} />
            </button>
            <button class="btn-icon" title="Export .tar archive" aria-label="Export .tar archive" onclick={() => store.exportMint(entry.target)}>
              <Download size={14} />
            </button>
            <button class="btn-icon del" title="Delete saved mint" aria-label="Delete saved mint" onclick={() => store.deleteMint(entry.target)}>
              <Trash2 size={14} />
            </button>
          </div>
        </div>
      {/each}
    {/if}
  </div>
</section>

<style>
  .drawer {
    position: fixed;
    left: 0;
    right: 0;
    bottom: 0;
    z-index: 30;
    height: calc(48px + env(safe-area-inset-bottom));
    padding-bottom: env(safe-area-inset-bottom);
    display: flex;
    flex-direction: column;
    background: var(--surface-1);
    border-top: 1px solid var(--border);
    box-shadow: 0 -12px 32px rgba(0, 0, 0, 0.35);
    transition: height 260ms cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  .drawer.open { height: min(65vh, 440px); }
  .handle {
    flex: none;
    height: 48px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 24px;
    color: var(--dim);
    text-align: left;
    transition: color 120ms, background 120ms;
  }
  .handle:hover { color: var(--head); background: var(--surface-2); }
  .title { font-weight: 600; color: var(--fg); }
  .chev { margin-left: auto; display: grid; transition: transform 260ms; }
  .open .chev { transform: rotate(180deg); }

  .list { flex: 1; min-height: 0; overflow-y: auto; padding: 0 24px 14px; }
  .empty { color: var(--dim); padding: 22px 0; font-size: 12.5px; }

  .row {
    display: grid;
    grid-template-columns: 28px 150px minmax(0, 1fr) 70px auto;
    align-items: center;
    gap: 16px;
    padding: 11px 12px;
    margin: 0 -12px;
    border-radius: var(--r-md);
    transition: background 90ms;
  }
  .row:hover { background: var(--surface-2); }

  .ring { width: 26px; height: 26px; }
  .trk { fill: none; stroke: var(--border); stroke-width: 2.5; }
  .arc { fill: none; stroke: var(--accent); stroke-width: 2.5; stroke-linecap: round; }
  .arc.done { stroke: var(--locked); }

  .meta { display: flex; flex-direction: column; min-width: 0; }
  .tgt { color: var(--head); font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cnt { font-size: 11px; color: var(--dim); }

  .addr { font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .matched { color: var(--locked); font-weight: 600; }
  .rest { color: var(--muted); }
  .age { font-size: 12px; color: var(--dim); text-align: right; }

  .acts { display: flex; align-items: center; justify-content: flex-end; gap: 4px; min-width: 150px; }
  .del:hover { color: #e69d79; }

  @media (max-width: 780px) {
    .row { grid-template-columns: 28px minmax(0, 1fr) auto; }
    .addr, .age { display: none; }
    .acts { min-width: 0; }
    .handle, .list { padding-left: 14px; padding-right: 14px; }
  }
  @media (prefers-reduced-motion: reduce) {
    .drawer, .chev { transition: none; }
  }
</style>
