<script lang="ts">
  import { Copy, Check } from '@lucide/svelte';
  import { onDestroy } from 'svelte';
  import { store } from '../lib/store.svelte';

  const ALPHABET = 'abcdefghijklmnopqrstuvwxyz234567';
  const LEN = 56;

  let copied = $state(false);
  let tumble = $state<string[]>(Array.from({ length: LEN }, () => '·'));
  let input = $state<HTMLInputElement>();
  let rafId = 0;
  let lastTick = 0;

  const snap = $derived(store.snapshot);
  const prefix = $derived(snap.active ? snap.target : store.normalizedTarget);
  const n = $derived(prefix.length);
  const w = $derived(Math.max(n, 5));
  const best = $derived(snap.best_address ? snap.best_address.replace(/\.onion$/, '') : '');
  const bestFresh = $derived(best !== '' && store.normalizedTarget === snap.target);
  const bestLen = $derived(snap.best_len);

  function frame(now: number) {
    if (now - lastTick >= 50) {
      lastTick = now;
      tumble = Array.from({ length: LEN }, () => ALPHABET[Math.floor(Math.random() * 32)]);
    }
    if (snap.active) rafId = requestAnimationFrame(frame);
  }

  $effect(() => {
    if (snap.active) {
      rafId = requestAnimationFrame(frame);
      return () => cancelAnimationFrame(rafId);
    }
    cancelAnimationFrame(rafId);
  });

  onDestroy(() => cancelAnimationFrame(rafId));

  async function copyAddress() {
    if (!snap.best_address) return;
    await store.copyText(snap.best_address, 'Copied .onion address');
    copied = true;
    setTimeout(() => (copied = false), 1500);
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !store.busy && !store.targetError) {
      e.preventDefault();
      void store.startMint();
    }
  }
</script>

<div class="field" class:running={snap.active} class:done={snap.completed && !snap.active}>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="inner" role="presentation" onclick={() => input?.focus()}>
    <div class="line mono" style:--n={w}>
      {#if snap.active}
        {#each Array.from({ length: LEN }, (_, i) => i) as idx (idx)}
          {#if idx < bestLen}
            <span class="ch locked">{best[idx] ?? prefix[idx]}</span>
          {:else if idx < n}
            <span class="ch hunting">{tumble[idx]}</span>
          {:else}
            <span class="ch tail">{tumble[idx]}</span>
          {/if}
        {/each}
      {:else}
        <input
          bind:this={input}
          bind:value={store.targetInput}
          class="prefix"
          class:bad={Boolean(store.targetError)}
          maxlength={52}
          placeholder="onyun"
          spellcheck="false"
          autocomplete="off"
          aria-label="Target prefix"
          onkeydown={handleKeydown}
        />{#each Array.from({ length: LEN - w }, (_, i) => w + i) as idx (idx)}<span class="ch tail">{bestFresh ? (best[idx] ?? '·') : '·'}</span>{/each}
      {/if}<span class="tld">.onion</span>
    </div>

    <button
      type="button"
      class="btn-icon copy"
      disabled={!snap.best_address}
      onclick={(e) => { e.stopPropagation(); copyAddress(); }}
      title="Copy full .onion address"
      aria-label="Copy full .onion address"
    >
      {#if copied}<Check size={15} />{:else}<Copy size={15} />{/if}
    </button>
  </div>
</div>

<style>
  .field {
    position: relative;
    isolation: isolate;
    width: fit-content;
    max-width: 100%;
    margin: 0 auto;
    padding: 1px;
    border-radius: var(--r-lg);
    background: var(--border);
    transition: background 200ms;
  }
  .field.running {
    background: conic-gradient(
      from var(--orbit),
      transparent 0 55%,
      rgba(194, 168, 108, 0.25) 75%,
      var(--accent-hover) 98%,
      transparent 100%
    );
    animation: orbit 3.6s linear infinite;
  }
  .field.running::before {
    content: '';
    position: absolute;
    inset: -2px;
    z-index: -1;
    border-radius: inherit;
    background: inherit;
    filter: blur(12px);
    opacity: 0.55;
  }
  .field.done {
    background: rgba(175, 185, 122, 0.5);
    box-shadow: 0 0 22px rgba(175, 185, 122, 0.15);
  }
  @keyframes orbit { to { --orbit: 360deg; } }

  .inner {
    position: relative;
    background: var(--surface-0);
    border-radius: calc(var(--r-lg) - 1px);
    padding: 32px;
    cursor: text;
    user-select: none;
  }
  .line {
    font-size: clamp(1rem, 2vw, 1.5rem);
    line-height: 1.7;
    letter-spacing: 0.04em;
    word-break: break-all;
  }
  .ch { display: inline-block; }
  .ch.locked { color: var(--locked); font-weight: 600; text-shadow: 0 0 12px rgba(175, 185, 122, 0.3); }
  .ch.hunting { color: var(--accent-hover); }
  .ch.tail { color: var(--dim); opacity: 0.5; }
  .tld { color: var(--muted); }

  .prefix {
    width: calc(var(--n) * (1ch + 0.04em));
    max-width: 100%;
    font: inherit;
    letter-spacing: inherit;
    font-variant-ligatures: none;
    color: var(--accent-hover);
    background: transparent;
    border: 0;
    outline: 0;
    padding: 0;
    vertical-align: baseline;
    caret-color: var(--accent-hover);
    user-select: text;
  }
  .prefix::placeholder { color: var(--muted); }
  .prefix.bad { color: #e69d79; }

  .copy { position: absolute; top: 6px; right: 6px; width: 26px; height: 26px; padding: 4px; }
  .copy:disabled { opacity: 0.35; cursor: default; }

  @media (max-width: 600px) {
    .inner { padding: 20px; }
    .copy { top: 4px; right: 4px; }
  }
</style>
