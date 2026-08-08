<script lang="ts">
  import { onMount } from "svelte";
  import { confirm } from "@tauri-apps/plugin-dialog";
  import { t } from "./i18n.svelte";
  import * as ipc from "./ipc";
  import type { PresetSummary, RuntimeConfig } from "./ipc";

  let {
    config,
    isDirty,
    onApplyLoadedConfig,
    onResetToDefault,
  }: {
    config: RuntimeConfig;
    isDirty: boolean;
    onApplyLoadedConfig: (config: RuntimeConfig) => void;
    onResetToDefault: () => void;
  } = $props();

  let presets: PresetSummary[] = $state([]);
  let newPresetName = $state("");
  let status = $state("");
  let busy = $state(false);

  async function refreshPresets() {
    try {
      presets = await ipc.listPresets();
    } catch (e) {
      status = `Failed to list presets: ${e}`;
    }
  }

  onMount(() => {
    refreshPresets();
  });

  async function handleSave() {
    if (busy) return;
    const name = newPresetName.trim() || "Preset";
    busy = true;
    try {
      const collidingName = await ipc.checkPresetCollision(name);
      if (collidingName && !(await confirm(t("presets.confirmOverwrite", { name: collidingName })))) return;
      await ipc.savePreset(name, config);
      newPresetName = "";
      status = `Saved "${name}"`;
      await refreshPresets();
    } catch (e) {
      status = `Save failed: ${e}`;
    } finally {
      busy = false;
    }
  }

  async function handleLoad(preset: PresetSummary) {
    if (busy) return;
    busy = true;
    try {
      if (isDirty && !(await confirm(t("presets.confirmDiscard", { name: preset.name })))) return;
      const payload = await ipc.loadPreset(preset.id);
      onApplyLoadedConfig(payload.config);
      status = `Loaded "${preset.name}"`;
    } catch (e) {
      status = `Load failed: ${e}`;
    } finally {
      busy = false;
    }
  }

  async function handleLoadDefault() {
    if (busy) return;
    busy = true;
    try {
      const defaultName = t("presets.defaultName");
      if (isDirty && !(await confirm(t("presets.confirmDiscard", { name: defaultName })))) return;
      onResetToDefault();
      status = `Loaded "${defaultName}"`;
    } finally {
      busy = false;
    }
  }

  async function handleDelete(preset: PresetSummary) {
    if (busy) return;
    busy = true;
    try {
      if (!(await confirm(t("presets.confirmDelete", { name: preset.name })))) return;
      await ipc.deletePreset(preset.id);
      status = `Deleted "${preset.name}"`;
      await refreshPresets();
    } catch (e) {
      status = `Delete failed: ${e}`;
    } finally {
      busy = false;
    }
  }

  async function handleExport(preset: PresetSummary) {
    if (busy) return;
    busy = true;
    try {
      const exported = await ipc.exportPreset(preset.id);
      if (exported) status = `Exported "${preset.name}"`;
    } catch (e) {
      status = `Export failed: ${e}`;
    } finally {
      busy = false;
    }
  }

  async function handleImport() {
    if (busy) return;
    busy = true;
    try {
      const payload = await ipc.importPreset();
      if (!payload) return;
      const collidingName = await ipc.checkPresetCollision(payload.name);
      if (collidingName && !(await confirm(t("presets.confirmOverwrite", { name: collidingName })))) return;
      await ipc.savePreset(payload.name, payload.config);
      status = `Imported "${payload.name}"`;
      await refreshPresets();
    } catch (e) {
      status = `Import failed: ${e}`;
    } finally {
      busy = false;
    }
  }

  async function handleOpenFolder() {
    if (busy) return;
    busy = true;
    try {
      await ipc.openPresetsFolder();
    } catch (e) {
      status = `Failed to open presets folder: ${e}`;
    } finally {
      busy = false;
    }
  }
</script>

<div class="panel">
  <div class="subhead">{t("presets.title")}</div>

  <div class="save-row">
    <input
      class="input"
      placeholder={t("presets.namePlaceholder")}
      aria-label={t("presets.namePlaceholder")}
      bind:value={newPresetName}
    />
    <button class="btn primary" disabled={busy} onclick={handleSave}>{t("presets.saveNew")}</button>
  </div>

  <div class="preset-list">
    <div class="preset-row">
      <div class="preset-info">
        <strong>{t("presets.defaultName")}</strong>
        <span class="tag">{t("presets.defaultBadge")}</span>
      </div>
      <div class="preset-actions">
        <button class="btn small" disabled={busy} onclick={handleLoadDefault}>{t("presets.load")}</button>
      </div>
    </div>

    {#each presets as preset (preset.id)}
      <div class="preset-row">
        <div class="preset-info">
          <strong>{preset.name}</strong>
        </div>
        <div class="preset-actions">
          <button class="btn small" disabled={busy} onclick={() => handleLoad(preset)}>{t("presets.load")}</button>
          <button class="btn small" disabled={busy} onclick={() => handleExport(preset)}>{t("presets.export")}</button>
          <button class="btn small danger" disabled={busy} onclick={() => handleDelete(preset)}>{t("presets.delete")}</button>
        </div>
      </div>
    {:else}
      <div class="muted small">{t("presets.empty")}</div>
    {/each}
  </div>

  <div class="footer-row">
    <button class="btn" disabled={busy} onclick={handleImport}>{t("presets.import")}</button>
    <button class="btn" disabled={busy} onclick={handleOpenFolder}>{t("presets.openFolder")}</button>
    {#if status}
      <span class="muted small">{status}</span>
    {/if}
  </div>
</div>

<style>
  .panel {
    padding: 1.5rem;
    display: flex;
    flex-direction: column;
    gap: 1rem;
    max-width: 40rem;
  }
  .subhead {
    font-weight: 600;
  }
  .save-row {
    display: flex;
    gap: 0.5rem;
  }
  .input {
    flex: 1;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: rgba(255, 255, 255, 0.02);
    color: var(--text);
    padding: 0.5rem 0.75rem;
  }
  .preset-list {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  .preset-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    background: rgba(255, 255, 255, 0.02);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 0.6rem 0.9rem;
  }
  .preset-info {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .tag {
    font-size: 0.7rem;
    color: var(--text-muted);
    background: rgba(255, 255, 255, 0.04);
    border-radius: 3px;
    padding: 0.1rem 0.4rem;
  }
  .preset-actions {
    display: flex;
    gap: 0.4rem;
  }
  .footer-row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }
  .muted.small {
    color: var(--text-muted);
    font-size: 0.8rem;
  }
  .btn {
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: rgba(255, 255, 255, 0.03);
    color: var(--text);
    padding: 0.4rem 0.9rem;
    cursor: pointer;
  }
  .btn.small {
    font-size: 0.8rem;
    padding: 0.3rem 0.6rem;
  }
  .btn.primary {
    background: var(--primary);
    border-color: var(--primary);
    color: white;
  }
  .btn.danger {
    border-color: color-mix(in srgb, var(--danger) 50%, transparent);
    color: var(--danger);
  }
</style>
