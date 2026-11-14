<script lang="ts">
  import { Tween } from 'svelte/motion';
  import { Play, Square, AlertCircle } from '@lucide/svelte';
  import { formatCount, formatDuration, formatHashrate, store } from '../lib/store.svelte';
  import OnionDisplay from './OnionDisplay.svelte';

  const snap = $derived(store.snapshot);
  const maxThreads = $derived(Math.max(1, store.cpuThreads));
  const pct = $derived(maxThreads > 1 ? ((store.threadsInput - 1) / (maxThreads - 1)) * 100 : 100);
  const targetLen = $derived(snap.target_len || store.normalizedTarget.length);

  const displayHashes = new Tween(0, { duration: 110, easing: (t) => t });

  $effect(() => {
    const total = snap.total_hashes;
    const reset = !snap.active || total < displayHashes.target;
    void displayHashes.set(total, { duration: reset ? 0 : 110 });
  });

  function handleSubmit(e: SubmitEvent) {
    e.preventDefault();
    if (snap.active) void store.stopMint();
    else void store.startMint();
  }
</script>

<div class="mint-page">
  <header class="brand-head">
    <h1 class="wordmark" class:live={snap.active}>Onyun</h1>
  </header>

  <OnionDisplay />

  <div class="error-slot">
    {#if store.targetError && !snap.active}
      <p class="field-error"><AlertCircle size={13} /><span>{store.targetError}</span></p>
    {/if}
  </div>

  <section class="panel">
    <form class="actions" onsubmit={handleSubmit}>
      <label class="threads">
        <span class="lab">Threads</span>
        <input
          class="slider"
          type="range"
          min={1}
          max={maxThreads}
          style:--pct="{pct}%"
          disabled={snap.active}
          bind:value={store.threadsInput}
        />
        <span class="mono val">{store.threadsInput}</span>
      </label>

      {#if snap.active}
        <button type="submit" class="go go-stop" disabled={store.busy}>
          <Square size={14} /><span>Stop</span>
        </button>
      {:else}
        <button type="submit" class="go go-start" disabled={store.busy || Boolean(store.targetError)}>
          <Play size={14} /><span>Start</span>
        </button>
      {/if}
    </form>

    <div class="divider"></div>

    <section class="stats">
      <div class="stat-cell hashrate-cell">
        <span class="lab">Hashrate</span>
        <span class="mono stat-val accent">{formatHashrate(snap.hashes_per_sec)}</span>
      </div>
      <div class="stat-cell"><span class="lab">Remaining</span><span class="mono stat-val">{formatDuration(snap.eta_secs)}</span></div>
      <div class="stat-cell"><span class="lab">Match</span><span class="mono stat-val">{snap.best_len}/{targetLen}</span></div>
      <div class="stat-cell"><span class="lab">Keys</span><span class="mono stat-val">{formatCount(Math.round(displayHashes.current))}</span></div>
      <div class="stat-cell"><span class="lab">Elapsed</span><span class="mono stat-val">{formatDuration(snap.elapsed_secs)}</span></div>
    </section>
  </section>
</div>

<style>
  .mint-page { max-width: 920px; margin: 0 auto; display: flex; flex-direction: column; gap: 14px; }
  .brand-head { display: flex; align-items: center; justify-content: center; margin-bottom: 4px; }
  .wordmark {
    font-family: var(--font-wordmark);
    font-size: clamp(64px, 9vw, 96px);
    line-height: 0.95;
    letter-spacing: -0.01em;
    color: var(--head);
  }
  .wordmark.live {
    color: var(--head);
    filter: drop-shadow(0 0 14px rgba(194, 168, 108, 0.18));
  }
  .error-slot { min-height: 18px; display: flex; justify-content: center; }
  .field-error { display: inline-flex; align-items: center; gap: 6px; font-size: 12px; color: #e69d79; }

  .panel {
    background: rgba(24, 24, 24, 0.85);
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    padding: 18px 24px 20px;
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.28);
  }
  .divider {
    height: 1px;
    background: var(--border-subtle);
    margin: 16px -24px 18px;
  }

  .actions { display: flex; align-items: center; justify-content: space-between; gap: 24px; }
  .threads { display: flex; align-items: center; gap: 14px; flex: 1; min-width: 0; }
  .lab { font-size: 12px; color: var(--dim); }
  .val { font-size: 13px; color: var(--accent); min-width: 2ch; text-align: right; }

  .go {
    height: 36px;
    min-width: 108px;
    padding: 0 18px;
    border-radius: var(--r-sm);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    font-size: 13px;
    font-weight: 500;
    border: 1px solid transparent;
    transition: background 140ms, border-color 140ms, color 140ms, box-shadow 140ms;
    box-shadow: var(--shadow-raised);
  }
  .go:disabled { opacity: 0.45; cursor: not-allowed; }
  .go-start {
    background: rgba(194, 168, 108, 0.1);
    border-color: rgba(194, 168, 108, 0.45);
    color: var(--head);
  }
  .go-start:hover:not(:disabled) {
    background: rgba(194, 168, 108, 0.18);
    border-color: var(--accent);
    box-shadow: 0 0 14px rgba(194, 168, 108, 0.18);
  }
  .go-stop {
    background: rgba(192, 109, 68, 0.1);
    border-color: rgba(192, 109, 68, 0.45);
    color: #f0b396;
  }
  .go-stop:hover:not(:disabled) {
    background: rgba(192, 109, 68, 0.18);
    border-color: var(--danger);
    box-shadow: 0 0 14px rgba(192, 109, 68, 0.18);
  }

  .slider {
    -webkit-appearance: none;
    appearance: none;
    width: 220px;
    height: 20px;
    background: transparent;
    cursor: pointer;
  }
  .slider::-webkit-slider-runnable-track {
    height: 3px;
    border-radius: 2px;
    background: linear-gradient(to right, var(--accent) var(--pct), var(--border) var(--pct));
  }
  .slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    width: 14px;
    height: 14px;
    margin-top: -5.5px;
    border-radius: 50%;
    background: var(--bg);
    border: 2px solid var(--accent);
    box-shadow: 0 0 10px rgba(194, 168, 108, 0.42);
    transition: box-shadow 140ms;
  }
  .slider:hover:not(:disabled)::-webkit-slider-thumb {
    box-shadow: 0 0 0 4px var(--accent-soft), 0 0 14px rgba(194, 168, 108, 0.65);
  }
  .slider::-moz-range-track { height: 3px; border-radius: 2px; background: var(--border); }
  .slider::-moz-range-progress { height: 3px; border-radius: 2px; background: var(--accent); }
  .slider::-moz-range-thumb {
    width: 10px; height: 10px; border-radius: 50%;
    background: var(--bg); border: 2px solid var(--accent);
  }
  .slider:disabled { opacity: 0.45; cursor: not-allowed; }

  .stats {
    display: grid;
    grid-template-columns: repeat(5, minmax(0, 1fr));
    align-items: stretch;
    gap: 0;
    margin: -18px -24px -20px;
  }
  .stat-cell {
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    gap: 6px;
    padding: 18px 24px 22px;
    border-right: 1px solid var(--border-subtle);
  }
  .stat-cell:last-child { border-right: none; }
  .stat-val { font-size: 22px; font-weight: 500; line-height: 1.15; color: var(--head); }
  .stat-val.accent { color: var(--accent); }
  .stats .mono { font-variant-numeric: tabular-nums; }

  @media (pointer: coarse) {
    .slider { height: 32px; }
    .slider::-webkit-slider-thumb { width: 20px; height: 20px; margin-top: -8.5px; }
    .slider::-moz-range-thumb { width: 18px; height: 18px; }
  }

  @media (max-width: 720px) {
    .panel { padding: 16px; }
    .divider { margin: 14px -16px 16px; }
    .actions { flex-direction: column; gap: 14px; align-items: stretch; }
    .threads { width: 100%; }
    .slider { flex: 1; width: auto; min-width: 0; }
    .go { width: 100%; }
    .stats { grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 16px 0; margin: -16px -16px -16px; }
    .stat-cell { padding: 14px 16px 18px; }
    .stat-cell:nth-child(even) { border-right: none; }
    .hashrate-cell { grid-column: 1 / -1; border-right: none; }
    .stat-val { font-size: 20px; }
  }
</style>
