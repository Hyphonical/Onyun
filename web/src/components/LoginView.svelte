<script lang="ts">
  import { ArrowRight } from '@lucide/svelte';
  import { store } from '../lib/store.svelte';

  let username = $state('');
  let password = $state('');
  let busy = $state(false);
  let error = $state<string | null>(null);

  async function handleSubmit(e: SubmitEvent) {
    e.preventDefault();
    if (busy) return;
    busy = true;
    error = null;
    try {
      await store.submitLogin(username, password);
    } catch (err) {
      error = err instanceof Error ? err.message : 'Sign in failed';
    } finally {
      busy = false;
    }
  }
</script>

<div class="standalone-stage">
  <form class="auth-card" onsubmit={handleSubmit}>
    <div class="auth-brand">
      <img src="/branding/icon.svg" alt="Onyun" class="setup-logo" />
      <span class="brand-word">Onyun</span>
    </div>

    <div>
      <h1 class="editorial-title">Sign in</h1>
      <p class="auth-sub">Enter your Onyun credentials to access the minter.</p>
    </div>

    {#if error}
      <p class="auth-error">{error}</p>
    {/if}

    <label class="form-field">
      <span>Username</span>
      <input
        class="input"
        type="text"
        autocomplete="username"
        required
        bind:value={username}
      />
    </label>

    <label class="form-field">
      <span>Password</span>
      <input
        class="input"
        type="password"
        autocomplete="current-password"
        required
        bind:value={password}
      />
    </label>

    <button type="submit" class="btn btn-primary submit-btn" disabled={busy}>
      <span>{busy ? 'Signing in…' : 'Sign in'}</span>
      <ArrowRight size={14} />
    </button>
  </form>
</div>

<style>
  .standalone-stage {
    min-height: 100%;
    display: grid;
    place-content: center;
    padding: 24px;
  }

  .auth-card {
    width: min(380px, 92vw);
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    padding: 28px 26px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .auth-brand {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .setup-logo {
    width: 30px;
    height: 30px;
    border-radius: 8px;
  }

  .brand-word {
    font-family: var(--font-display);
    font-size: 20px;
    color: var(--head);
  }

  .auth-sub {
    color: var(--dim);
    font-size: 12.5px;
    margin-top: 6px;
  }

  .auth-error {
    background: var(--danger-soft);
    border: 1px solid rgba(192, 109, 68, 0.45);
    color: #ebb093;
    border-radius: var(--r-sm);
    padding: 8px 11px;
    font-size: 12px;
  }

  .form-field {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 12px;
    color: var(--dim);
  }

  .submit-btn {
    margin-top: 4px;
    height: 38px;
  }
</style>
