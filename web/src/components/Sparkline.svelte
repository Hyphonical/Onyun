<script lang="ts">
  import { store } from '../lib/store.svelte';

  const W = 220;
  const H = 38;

  const points = $derived.by(() => {
    const vals = store.rateHistory;
    if (vals.length < 2) return '';
    const max = Math.max(...vals, 0.1);
    const min = Math.min(...vals, 0);
    const span = Math.max(0.05, max - min);
    return vals
      .map((v, i) => {
        const x = (i / (vals.length - 1)) * W;
        const y = H - 3 - ((v - min) / span) * (H - 6);
        return `${x.toFixed(1)},${y.toFixed(1)}`;
      })
      .join(' ');
  });

  const areaPoints = $derived(points ? `0,${H} ${points} ${W},${H}` : '');
</script>

<svg viewBox="0 0 {W} {H}" class="sparkline" aria-hidden="true">
  {#if areaPoints}
    <polygon points={areaPoints} fill="var(--accent-soft)" />
    <polyline
      points={points}
      fill="none"
      stroke="var(--accent)"
      stroke-width="1.75"
      stroke-linecap="round"
      stroke-linejoin="round"
    />
  {:else}
    <line x1="0" y1={H - 4} x2={W} y2={H - 4} stroke="var(--border)" stroke-width="1" />
  {/if}
</svg>

<style>
  .sparkline {
    width: 100%;
    height: 38px;
    display: block;
  }
</style>
