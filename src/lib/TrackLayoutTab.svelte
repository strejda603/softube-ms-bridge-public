<script lang="ts">
  import { t } from "./i18n.svelte";
  import type { Row } from "./trackLayoutRows";
  import { clampInt, computeRestNumbers, ensureRowsLength } from "./trackLayoutRows";
  import { fetchMixingStationInformation, inferCountsFromInformation } from "./trackLayoutDetect";
  import TrackOrderEditor from "./TrackOrderEditor.svelte";

  const LAYOUT_TAB_STORAGE_KEY = "softubeMsBridge.layoutTab";

  let {
    inputRows,
    busRows,
    inputTotalCount,
    busTotalCount,
    wsUrl,
    onChange,
  }: {
    inputRows: Row[];
    busRows: Row[];
    inputTotalCount: number;
    busTotalCount: number;
    wsUrl: string;
    onChange: (next: {
      inputRows: Row[];
      busRows: Row[];
      inputTotalCount: number;
      busTotalCount: number;
    }) => void;
  } = $props();

  function safeLocalStorageGet(key: string): string | null {
    try {
      return localStorage.getItem(key);
    } catch {
      return null;
    }
  }
  function safeLocalStorageSet(key: string, value: string) {
    try {
      localStorage.setItem(key, value);
    } catch {
      // ignore
    }
  }

  let subTab = $state<"inputs" | "buses">(
    safeLocalStorageGet(LAYOUT_TAB_STORAGE_KEY) === "buses" ? "buses" : "inputs"
  );
  let detectStatus = $state("");
  let detecting = $state(false);
  let inputsTabButton: HTMLButtonElement | undefined = $state();
  let busesTabButton: HTMLButtonElement | undefined = $state();

  function selectSubTab(tab: "inputs" | "buses") {
    subTab = tab;
    safeLocalStorageSet(LAYOUT_TAB_STORAGE_KEY, tab);
  }

  function focusTab(tab: "inputs" | "buses") {
    (tab === "inputs" ? inputsTabButton : busesTabButton)?.focus();
  }

  function handleSubTabKeydown(e: KeyboardEvent) {
    if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
      e.preventDefault();
      const next = subTab === "inputs" ? "buses" : "inputs";
      selectSubTab(next);
      focusTab(next);
    } else if (e.key === "Home") {
      e.preventDefault();
      selectSubTab("inputs");
      focusTab("inputs");
    } else if (e.key === "End") {
      e.preventDefault();
      selectSubTab("buses");
      focusTab("buses");
    }
  }

  const inputRestCount = $derived(computeRestNumbers(inputRows, inputTotalCount).length);
  const busRestCount = $derived(computeRestNumbers(busRows, busTotalCount).length);

  function emit(
    partial: Partial<{
      inputRows: Row[];
      busRows: Row[];
      inputTotalCount: number;
      busTotalCount: number;
    }>
  ) {
    onChange({ inputRows, busRows, inputTotalCount, busTotalCount, ...partial });
  }

  async function handleDetectFromMixer() {
    if (detecting) return;
    detecting = true;
    detectStatus = t("trackLayout.detecting");
    try {
      const info = await fetchMixingStationInformation(wsUrl || "ws://localhost:8080");
      const { inputCount, busCount } = inferCountsFromInformation(info);

      const inputCustomCount = clampInt(inputRows.length, 0, inputCount);
      const busCustomCount = clampInt(busRows.length, 0, busCount);

      emit({
        inputTotalCount: inputCount,
        busTotalCount: busCount,
        inputRows: ensureRowsLength(inputRows, inputCustomCount, inputCount),
        busRows: ensureRowsLength(busRows, busCustomCount, busCount),
      });
      detectStatus = t("trackLayout.detected", { inputs: inputCount, buses: busCount });
    } catch (e) {
      detectStatus = t("trackLayout.detectFailed", {
        error: String((e as Error)?.message ?? e),
      });
    } finally {
      detecting = false;
    }
  }
</script>

<div class="track-layout">
  <div class="header-row">
    <div class="title">{t("trackLayout.title")}</div>
    <div class="detect-group">
      <button class="btn" disabled={detecting} onclick={handleDetectFromMixer}>
        {t("trackLayout.readFromMixer")}
      </button>
      {#if detectStatus}
        <span class="muted small">{detectStatus}</span>
      {/if}
    </div>
  </div>

  <div class="sub-tabs" role="tablist" aria-label={t("trackLayout.tabsAriaLabel")}>
    <button
      bind:this={inputsTabButton}
      role="tab"
      aria-selected={subTab === "inputs"}
      tabindex={subTab === "inputs" ? 0 : -1}
      class="sub-tab"
      class:active={subTab === "inputs"}
      onclick={() => selectSubTab("inputs")}
      onkeydown={handleSubTabKeydown}
    >
      {t("trackLayout.tabInputs")}
      {#if inputRestCount > 0}<span class="badge">{inputRestCount}</span>{/if}
    </button>
    <button
      bind:this={busesTabButton}
      role="tab"
      aria-selected={subTab === "buses"}
      tabindex={subTab === "buses" ? 0 : -1}
      class="sub-tab"
      class:active={subTab === "buses"}
      onclick={() => selectSubTab("buses")}
      onkeydown={handleSubTabKeydown}
    >
      {t("trackLayout.tabBuses")}
      {#if busRestCount > 0}<span class="badge">{busRestCount}</span>{/if}
    </button>
  </div>

  <div class="sub-panel" role="tabpanel" hidden={subTab !== "inputs"}>
    <TrackOrderEditor
      kind="input"
      rows={inputRows}
      totalCount={inputTotalCount}
      onRowsChange={(rows) => emit({ inputRows: rows })}
    />
  </div>
  <div class="sub-panel" role="tabpanel" hidden={subTab !== "buses"}>
    <TrackOrderEditor
      kind="bus"
      rows={busRows}
      totalCount={busTotalCount}
      onRowsChange={(rows) => emit({ busRows: rows })}
    />
  </div>
</div>

<style>
  .track-layout {
    padding: 1.5rem;
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }
  .header-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .title {
    font-weight: 600;
  }
  .detect-group {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }
  .sub-tabs {
    display: flex;
    gap: 0.25rem;
    border-bottom: 1px solid var(--border);
  }
  .sub-tab {
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    color: var(--text-muted);
    padding: 0.5rem 0.75rem;
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }
  .sub-tab.active {
    color: var(--text);
    border-bottom-color: var(--primary);
  }
  .badge {
    background: var(--primary-bg);
    color: var(--primary);
    border-radius: 999px;
    padding: 0 0.4rem;
    font-size: 0.7rem;
  }
  .btn {
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: rgba(255, 255, 255, 0.03);
    color: var(--text);
    padding: 0.4rem 0.9rem;
    cursor: pointer;
  }
  .muted.small {
    color: var(--text-muted);
    font-size: 0.8rem;
  }
</style>
