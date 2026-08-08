<script module lang="ts">
  export type TabId = "connection" | "trackLayout" | "sendsColors" | "presets";
</script>

<script lang="ts">
  import { t } from "./i18n.svelte";

  let { active, onSelect }: { active: TabId; onSelect: (id: TabId) => void } = $props();

  const tabs: { id: TabId; labelKey: string }[] = [
    { id: "connection", labelKey: "tabs.connection" },
    { id: "trackLayout", labelKey: "tabs.trackLayout" },
    { id: "sendsColors", labelKey: "tabs.sendsColors" },
    { id: "presets", labelKey: "tabs.presets" },
  ];
</script>

<div class="tab-bar" role="tablist">
  {#each tabs as tab (tab.id)}
    <button
      role="tab"
      aria-selected={active === tab.id}
      class="tab"
      class:active={active === tab.id}
      onclick={() => onSelect(tab.id)}
    >
      {t(tab.labelKey)}
    </button>
  {/each}
</div>

<style>
  .tab-bar {
    display: flex;
    gap: 0.25rem;
    padding: 0 1rem;
    border-bottom: 1px solid var(--border);
  }
  .tab {
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    color: var(--text-muted);
    padding: 0.75rem 1rem;
    cursor: pointer;
  }
  .tab.active {
    color: var(--text);
    border-bottom-color: var(--primary);
  }
</style>
