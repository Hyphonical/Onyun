<script lang="ts">
  import { Tween } from 'svelte/motion';
  import { Play, Square, AlertCircle } from '@lucide/svelte';
  import { formatCount, formatDuration, formatHashrate, store } from '../lib/store.svelte';
  import SafeDisplay from './SafeDisplay.svelte';

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
  <SafeDisplay />

  <div class="error-slot">
    {#if store.targetError && !snap.active}
      <p class="field-error"><AlertCircle size={13} /><span>{store.targetError}</span></p>
    {/if}
  </div>

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
      <button type="submit" class="btn btn-danger go" disabled={store.busy}>
        <Square size={14} /><span>Stop</span>
      </button>
    {:else}
      <button type="submit" class="btn btn-primary go" disabled={store.busy || Boolean(store.targetError)}>
        <Play size={14} /><span>Start</span>
      </button>
    {/if}
  </form>

  <section class="stats">
    <div><span class="lab">Hashrate</span><span class="mono big accent">{formatHashrate(snap.hashes_per_sec)}</span></div>
    <div><span class="lab">Remaining</span><span class="mono big">{formatDuration(snap.eta_secs)}</span></div>
    <div><span class="lab">Match</span><span class="mono mid">{snap.best_len}/{targetLen}</span></div>
    <div><span class="lab">Keys</span><span class="mono mid">{formatCount(Math.round(displayHashes.current))}</span></div>
    <div><span class="lab">Elapsed</span><span class="mono mid">{formatDuration(snap.elapsed_secs)}</span></div>
  </section>
</div>

<style>
  .mint-page { max-width: 1000px; margin: 0 auto; display: flex; flex-direction: column; gap: 14px; }
  .error-slot { min-height: 18px; display: flex; justify-content: center; }
  .field-error { display: inline-flex; align-items: center; gap: 6px; font-size: 12px; color: #e69d79; }

  .actions { display: flex; align-items: center; justify-content: center; gap: 32px; }
  .threads { display: flex; align-items: center; gap: 12px; }
  .lab { font-size: 12px; color: var(--dim); }
  .val { font-size: 13px; color: var(--accent); min-width: 2ch; text-align: right; }
  .go { height: 38px; min-width: 112px; padding: 0 20px; }

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
    transition: box-shadow 120ms;
  }
  .slider:hover:not(:disabled)::-webkit-slider-thumb { box-shadow: 0 0 0 4px var(--accent-soft); }
  .slider::-moz-range-track { height: 3px; border-radius: 2px; background: var(--border); }
  .slider::-moz-range-progress { height: 3px; border-radius: 2px; background: var(--accent); }
  .slider::-moz-range-thumb {
    width: 10px; height: 10px; border-radius: 50%;
    background: var(--bg); border: 2px solid var(--accent);
  }
  .slider:disabled { opacity: 0.45; cursor: not-allowed; }

  .stats {
    display: flex;
    justify-content: center;
    align-items: flex-end;
    gap: 44px;
    flex-wrap: wrap;
    margin-top: 18px;
  }
  .stats div { display: flex; flex-direction: column; gap: 3px; }
  .big { font-size: 30px; font-weight: 500; line-height: 1.1; color: var(--head); }
  .big.accent { color: var(--accent); }
  .mid { font-size: 16px; color: var(--fg); }
  .stats .mono { font-variant-numeric: tabular-nums; }

  @media (pointer: coarse) {
    .slider { height: 32px; }
    .slider::-webkit-slider-thumb { width: 20px; height: 20px; margin-top: -8.5px; }
    .slider::-moz-range-thumb { width: 18px; height: 18px; }
  }

  @media (max-width: 600px) {
    .actions { flex-direction: column; gap: 16px; align-items: stretch; }
    .threads { width: 100%; }
    .slider { flex: 1; width: auto; min-width: 0; }
    .go { width: 100%; }
    .stats { gap: 18px 28px; margin-top: 10px; }
    .big { font-size: 24px; }
  }
</style>
