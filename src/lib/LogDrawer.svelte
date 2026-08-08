<script lang="ts">
  import { classifyLogLine } from "./logClassify";
  import { t } from "./i18n.svelte";

  let { lines, hidden = false }: { lines: string[]; hidden?: boolean } = $props();

  let collapsed = $state(false);
  let autoScroll = $state(true);
  let logEl: HTMLDivElement | undefined = $state();

  $effect(() => {
    // Re-run whenever `lines` changes; scroll to bottom if auto-scroll is on.
    // Assumes the caller passes a new array reference on update, not an in-place push/shift.
    void lines.length;
    if (autoScroll && logEl) {
      logEl.scrollTop = logEl.scrollHeight;
    }
  });
</script>

{#if !hidden}
  <section class="log-drawer" class:collapsed>
    <div class="log-drawer-header">
      <button
        class="btn-icon"
        aria-expanded={!collapsed}
        aria-label={t("console.title")}
        onclick={() => (collapsed = !collapsed)}
      >
        {collapsed ? "▸" : "▾"}
      </button>
      <span>{t("console.title")}</span>
      <div class="log-drawer-actions">
        <label class="checkbox">
          <input type="checkbox" bind:checked={autoScroll} />
          <span>{t("console.autoScroll")}</span>
        </label>
      </div>
    </div>
    {#if !collapsed}
      <div class="log" bind:this={logEl}>
        {#each lines as line, i (i)}
          <span class="log-line {classifyLogLine(line)}">{line}</span>
        {/each}
      </div>
    {/if}
  </section>
{/if}

<style>
  .log-drawer {
    border-top: 1px solid var(--border);
    background: var(--panel-2);
    flex-shrink: 0;
  }
  .log-drawer-header {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.5rem 1rem;
  }
  .log-drawer-actions {
    margin-left: auto;
  }
  .btn-icon {
    background: transparent;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
  }
  .log {
    height: 220px;
    overflow-y: auto;
    font-family: var(--mono);
    font-size: 0.8rem;
    padding: 0.5rem 1rem;
    white-space: pre-wrap;
  }
  .log-line {
    display: block;
  }
  .log-error {
    color: var(--danger);
  }
  .log-warn {
    color: var(--text-muted);
  }
  .log-gui,
  .log-mode {
    color: var(--primary);
  }
  .log-json {
    color: var(--text-muted);
  }
</style>
