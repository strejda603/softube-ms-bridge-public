<script lang="ts">
  import { t } from "./i18n.svelte";
  import SendsMappingPanel from "./SendsMappingPanel.svelte";
  import ColorsPanel from "./ColorsPanel.svelte";

  const SUB_TAB_STORAGE_KEY = "softubeMsBridge.sendsColorsTab";

  let {
    sends,
    busTotalCount,
    mainColor,
    busColor,
    onSendsChange,
    onColorsChange,
  }: {
    sends: number[];
    busTotalCount: number;
    mainColor: number;
    busColor: number;
    onSendsChange: (sends: number[]) => void;
    onColorsChange: (next: { mainColor: number; busColor: number }) => void;
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

  let subTab = $state<"sends" | "colors">(
    safeLocalStorageGet(SUB_TAB_STORAGE_KEY) === "colors" ? "colors" : "sends"
  );
  let sendsTabButton: HTMLButtonElement | undefined = $state();
  let colorsTabButton: HTMLButtonElement | undefined = $state();

  function selectSubTab(tab: "sends" | "colors") {
    subTab = tab;
    safeLocalStorageSet(SUB_TAB_STORAGE_KEY, tab);
  }

  function focusTab(tab: "sends" | "colors") {
    (tab === "sends" ? sendsTabButton : colorsTabButton)?.focus();
  }

  function handleSubTabKeydown(e: KeyboardEvent) {
    if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
      e.preventDefault();
      const next = subTab === "sends" ? "colors" : "sends";
      selectSubTab(next);
      focusTab(next);
    } else if (e.key === "Home") {
      e.preventDefault();
      selectSubTab("sends");
      focusTab("sends");
    } else if (e.key === "End") {
      e.preventDefault();
      selectSubTab("colors");
      focusTab("colors");
    }
  }
</script>

<div class="sends-colors">
  <div class="sub-tabs" role="tablist" aria-label={t("sendsColors.tabsAriaLabel")}>
    <button
      bind:this={sendsTabButton}
      role="tab"
      aria-selected={subTab === "sends"}
      tabindex={subTab === "sends" ? 0 : -1}
      class="sub-tab"
      class:active={subTab === "sends"}
      onclick={() => selectSubTab("sends")}
      onkeydown={handleSubTabKeydown}
    >
      {t("sendsColors.tabSends")}
    </button>
    <button
      bind:this={colorsTabButton}
      role="tab"
      aria-selected={subTab === "colors"}
      tabindex={subTab === "colors" ? 0 : -1}
      class="sub-tab"
      class:active={subTab === "colors"}
      onclick={() => selectSubTab("colors")}
      onkeydown={handleSubTabKeydown}
    >
      {t("sendsColors.tabColors")}
    </button>
  </div>

  <div class="sub-panel" role="tabpanel" hidden={subTab !== "sends"}>
    <SendsMappingPanel {sends} {busTotalCount} {onSendsChange} />
  </div>
  <div class="sub-panel" role="tabpanel" hidden={subTab !== "colors"}>
    <ColorsPanel {mainColor} {busColor} {onColorsChange} />
  </div>
</div>

<style>
  .sends-colors {
    display: flex;
    flex-direction: column;
  }
  .sub-tabs {
    display: flex;
    gap: 0.25rem;
    border-bottom: 1px solid var(--border);
    padding: 0 1.5rem;
  }
  .sub-tab {
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    color: var(--text-muted);
    padding: 0.5rem 0.75rem;
    cursor: pointer;
  }
  .sub-tab.active {
    color: var(--text);
    border-bottom-color: var(--primary);
  }
</style>
