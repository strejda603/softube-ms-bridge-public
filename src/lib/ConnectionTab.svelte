<script lang="ts">
  import { t } from "./i18n.svelte";

  let {
    wsUrl,
    logJson,
    meteringIntervalMs,
    onChange,
  }: {
    wsUrl: string;
    logJson: boolean;
    meteringIntervalMs: number;
    onChange: (next: { wsUrl: string; logJson: boolean; meteringIntervalMs: number }) => void;
  } = $props();

  function clampInterval(raw: number): number {
    if (!Number.isFinite(raw)) return 100;
    return Math.max(30, Math.min(1000, Math.trunc(raw)));
  }

  function emit(partial: Partial<{ wsUrl: string; logJson: boolean; meteringIntervalMs: number }>) {
    onChange({ wsUrl, logJson, meteringIntervalMs, ...partial });
  }
</script>

<div class="panel">
  <label class="label" for="wsUrl">{t("connection.wsUrlLabel")}</label>
  <input
    id="wsUrl"
    class="input"
    value={wsUrl}
    placeholder={t("connection.wsUrlPlaceholder")}
    onchange={(e) => emit({ wsUrl: (e.target as HTMLInputElement).value.trim() })}
  />

  <label class="label" for="metering">{t("connection.meteringIntervalLabel")}</label>
  <div class="row">
    <input
      id="metering"
      type="range"
      min="30"
      max="1000"
      step="1"
      value={meteringIntervalMs}
      oninput={(e) => emit({ meteringIntervalMs: clampInterval(Number((e.target as HTMLInputElement).value)) })}
    />
    <input
      class="input narrow"
      type="number"
      min="30"
      max="1000"
      step="1"
      value={meteringIntervalMs}
      oninput={(e) => emit({ meteringIntervalMs: clampInterval(Number((e.target as HTMLInputElement).value)) })}
    />
  </div>

  <label class="checkbox">
    <input
      type="checkbox"
      checked={logJson}
      onchange={(e) => emit({ logJson: (e.target as HTMLInputElement).checked })}
    />
    <span>{t("connection.logJsonLabel")}</span>
  </label>
</div>

<style>
  .panel {
    padding: 1.5rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    max-width: 32rem;
  }
  .label {
    font-size: 0.8rem;
    color: var(--text-muted);
  }
  .input {
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: rgba(255, 255, 255, 0.02);
    color: var(--text);
    padding: 0.5rem 0.75rem;
  }
  .input.narrow {
    width: 6rem;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }
  .checkbox {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
</style>
