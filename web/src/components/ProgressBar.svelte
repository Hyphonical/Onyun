<script lang="ts">
  import { store } from '../lib/store.svelte';

  const snap = $derived(store.snapshot);
  const pct = $derived(Math.min(100, Math.max(0, snap.progress * 100)));
  const ticks = $derived(
    snap.target_len > 0 ? Array.from({ length: snap.target_len }, (_, i) => i + 1) : []
  );

  const paceLabel = $derived.by(() => {
    if (!snap.active || snap.pace_ratio <= 0) return null;
    if (snap.pace_ratio < 0.75) return `${snap.pace_ratio.toFixed(2)}× lucky`;
    if (snap.pace_ratio > 1.5) return `${snap.pace_ratio.toFixed(1)}× overdue`;
    return `${snap.pace_ratio.toFixed(2)}× avg`;
  });
</script>

<div class="progress-panel">
  <div class="progress-top">
    <span class="progress-label">
      Logarithmic prefix progress
      {#if snap.target_len > 0}
        <span class="mono dim">({snap.best_len}/{snap.target_len} chars)</span>
      {/if}
    </span>
    <div class="progress-badges">
      {#if paceLabel}
        <span class="badge" class:accent={snap.pace_ratio <= 1.0} class:danger={snap.pace_ratio > 2.0}>
          {paceLabel}
        </span>
      {/if}
      <span class="mono pct">{pct.toFixed(1)}%</span>
    </div>
  </div>

  <div class="track" role="progressbar" aria-valuenow={Math.round(pct)} aria-valuemin={0} aria-valuemax={100}>
    <div class="fill" class:done={snap.completed} style:width="{pct}%"></div>
    {#each ticks as tick (tick)}
      {#if tick < snap.target_len}
        <span
          class="tick"
          class:passed={tick <= snap.best_len}
          style:left="{(tick / snap.target_len) * 100}%"
        ></span>
      {/if}
    {/each}
  </div>
</div>

<style>
  .progress-panel {
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
    padding: 14px 18px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .progress-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    font-size: 12px;
  }

  .progress-label {
    color: var(--fg);
    font-weight: 500;
  }

  .dim {
    color: var(--dim);
    margin-left: 6px;
  }

  .progress-badges {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .pct {
    color: var(--head);
    font-size: 12.5px;
    font-weight: 600;
  }

  .track {
    position: relative;
    height: 10px;
    border-radius: 999px;
    background: var(--surface-0);
    border: 1px solid var(--border-subtle);
    overflow: hidden;
  }

  .fill {
    height: 100%;
    background: linear-gradient(90deg, var(--accent), var(--accent-hover));
    border-radius: 999px;
    transition: width 110ms linear;
  }

  .fill.done {
    background: var(--locked);
  }

  .tick {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 1px;
    background: rgba(255, 255, 212, 0.16);
    pointer-events: none;
  }

  .tick.passed {
    background: rgba(20, 20, 20, 0.55);
  }
</style>
