<script lang="ts">
  import { t } from "./i18n.svelte";
  import { normalizeC1SendMapping } from "./sendsMapping";

  let {
    sends,
    busTotalCount,
    onSendsChange,
  }: {
    sends: number[];
    busTotalCount: number;
    onSendsChange: (sends: number[]) => void;
  } = $props();

  const busOptions = $derived(Array.from({ length: busTotalCount }, (_, i) => i + 1));

  function handleSendChange(index: number, value: string) {
    const next = sends.slice();
    next[index] = Number(value);
    onSendsChange(normalizeC1SendMapping(next, busTotalCount));
  }
</script>

<div class="panel">
  <div class="subhead">{t("sendsMapping.title")}</div>
  <div class="muted small">{t("sendsMapping.subhead")}</div>
  <div class="send-grid" aria-label={t("sendsMapping.ariaLabel")}>
    <!-- Fixed 0..5 literal: this panel always renders exactly 6 send slots (sends is
         guaranteed length-6 by normalizeC1SendMapping everywhere it's assigned), and
         sends[index] ?? index + 1 below is defensive, not load-bearing. -->
    {#each [0, 1, 2, 3, 4, 5] as index (index)}
      <label class="send-cell">
        <span>{t(`sendsMapping.send${index + 1}`)}</span>
        <select
          value={String(sends[index] ?? index + 1)}
          onchange={(e) => handleSendChange(index, (e.target as HTMLSelectElement).value)}
        >
          {#each busOptions as bus (bus)}
            <option value={String(bus)}>{t("sendsMapping.busOption", { bus })}</option>
          {/each}
        </select>
      </label>
    {/each}
  </div>
</div>

<style>
  .panel {
    padding: 1.5rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .subhead {
    font-weight: 600;
  }
  .muted.small {
    color: var(--text-muted);
    font-size: 0.8rem;
    margin-bottom: 0.5rem;
  }
  .send-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
    gap: 0.75rem;
    max-width: 40rem;
  }
  .send-cell {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    font-size: 0.8rem;
    color: var(--text-muted);
  }
  .send-cell select {
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: rgba(255, 255, 255, 0.02);
    color: var(--text);
    padding: 0.4rem 0.5rem;
  }
</style>
