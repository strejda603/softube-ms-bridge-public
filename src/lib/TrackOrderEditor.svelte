<script lang="ts">
  import { t } from "./i18n.svelte";
  import type { Row } from "./trackLayoutRows";
  import {
    canLinkAdjacent,
    clampInt,
    computeRestNumbers,
    ensureRowsLength,
    generateSequentialRows,
    insertRowAt,
    labelFor,
    labelForRest,
    linkRowsAt,
    moveArrayItem,
    removeRowAt,
    swapRows,
    unlinkRowAt,
  } from "./trackLayoutRows";

  let {
    kind,
    rows,
    totalCount,
    onRowsChange,
  }: {
    kind: "input" | "bus";
    rows: Row[];
    totalCount: number;
    onRowsChange: (rows: Row[]) => void;
  } = $props();

  const countLabelKey = $derived(
    kind === "input" ? "trackLayout.inputCountLabel" : "trackLayout.busCountLabel"
  );
  const orderAriaLabelKey = $derived(
    kind === "input" ? "trackLayout.inputOrderAriaLabel" : "trackLayout.busOrderAriaLabel"
  );
  const unorganizedAriaLabelKey = $derived(
    kind === "input"
      ? "trackLayout.unorganizedInputsAriaLabel"
      : "trackLayout.unorganizedBusesAriaLabel"
  );

  // Mirrors `rows.length` -- kept as separate local state (rather than reading `rows.length`
  // directly in the input's `value`) so typing digits before blur doesn't get clobbered by
  // this effect, while any change to `rows` from elsewhere (drag-drop, chip click, Generate)
  // still keeps the field in sync, matching `renderer.js`'s `syncCountsToState()` calls.
  // svelte-ignore state_referenced_locally -- intentional one-time seed, see comment above
  let countField = $state(rows.length);
  $effect(() => {
    countField = rows.length;
  });

  const restNumbers = $derived(computeRestNumbers(rows, totalCount));

  function handleCountChange() {
    const n = clampInt(countField, 0, totalCount);
    onRowsChange(ensureRowsLength(rows, n, totalCount));
  }

  function handleGenerate() {
    const n = clampInt(countField, 0, totalCount);
    onRowsChange(ensureRowsLength(generateSequentialRows(n), n, totalCount));
  }

  function handleChipClick(value: number) {
    onRowsChange(insertRowAt(rows, rows.length, value));
  }

  type DragPayload = { source: "row"; index: number } | { source: "chip"; value: number };
  let dragPayload: DragPayload | null = $state(null);

  function handleRowDragStart(index: number, e: DragEvent) {
    dragPayload = { source: "row", index };
    try {
      e.dataTransfer!.effectAllowed = "move";
    } catch {
      // ignore
    }
  }

  function handleChipDragStart(value: number, e: DragEvent) {
    dragPayload = { source: "chip", value };
    try {
      e.dataTransfer!.effectAllowed = "copyMove";
    } catch {
      // ignore
    }
  }

  function handleRowDrop(dropIndex: number, e: DragEvent) {
    e.preventDefault();
    e.stopPropagation();
    const payload = dragPayload;
    dragPayload = null;
    if (!payload) return;
    if (payload.source === "chip") {
      onRowsChange(insertRowAt(rows, dropIndex, payload.value));
      return;
    }
    onRowsChange(moveArrayItem(rows, payload.index, dropIndex));
  }

  function handleRestDrop(e: DragEvent) {
    e.preventDefault();
    const payload = dragPayload;
    dragPayload = null;
    if (!payload || payload.source !== "row") return;
    onRowsChange(removeRowAt(rows, payload.index));
  }
</script>

<div class="editor">
  <div class="count-row">
    <label class="label" for={`${kind}Count`}>{t(countLabelKey)}</label>
    <input
      id={`${kind}Count`}
      class="input narrow"
      type="number"
      min="0"
      max="512"
      bind:value={countField}
      onchange={handleCountChange}
    />
    <button class="btn small" title={t("trackLayout.generateTitle")} onclick={handleGenerate}>
      {t("trackLayout.generate")}
    </button>
  </div>

  <div class="columns">
    <div class="column">
      <div class="subhead">{t("trackLayout.customOrdering")}</div>
      <div class="muted small">{t("trackLayout.customOrderingHint")}</div>
      <!-- svelte-ignore a11y_no_static_element_interactions -- drop target, not a widget; see file-level note on ARIA roles -->
      <div
        class="order-list"
        aria-label={t(orderAriaLabelKey)}
        ondragover={(e) => e.preventDefault()}
        ondrop={(e) => handleRowDrop(rows.length, e)}
      >
        {#each rows as row, index (row.join("-"))}
          <!-- svelte-ignore a11y_no_static_element_interactions -- draggable row, not a widget; see file-level note on ARIA roles -->
          <div
            class="list-item"
            draggable="true"
            ondragstart={(e) => handleRowDragStart(index, e)}
            ondragover={(e) => e.preventDefault()}
            ondrop={(e) => handleRowDrop(index, e)}
            ondragend={() => (dragPayload = null)}
          >
            <div class="chip">
              <strong>{labelFor(kind, row)}</strong>
              <span class="muted">
                {row.length === 2 ? t("trackLayout.stereo") : t("trackLayout.mono")}
              </span>
            </div>
            <div class="row-actions">
              <button class="btn small" disabled={index === 0} onclick={() => onRowsChange(swapRows(rows, index, index - 1))}>
                ↑
              </button>
              <button
                class="btn small"
                disabled={index === rows.length - 1}
                onclick={() => onRowsChange(swapRows(rows, index, index + 1))}
              >
                ↓
              </button>
              {#if row.length === 2}
                <button class="btn small" onclick={() => onRowsChange(unlinkRowAt(rows, index))}>
                  {t("trackLayout.unlink")}
                </button>
              {:else}
                <button
                  class="btn small"
                  disabled={!canLinkAdjacent(rows, index)}
                  onclick={() => onRowsChange(linkRowsAt(rows, index))}
                >
                  {t("trackLayout.link")}
                </button>
              {/if}
              <button class="btn small danger" onclick={() => onRowsChange(removeRowAt(rows, index))}>
                ✕
              </button>
            </div>
          </div>
        {/each}
      </div>
    </div>

    <div class="column">
      <div class="subhead">{t("trackLayout.unorganized")}</div>
      <div class="muted small">{t("trackLayout.unorganizedHint")}</div>
      <!-- svelte-ignore a11y_no_static_element_interactions -- drop target, not a widget; see file-level note on ARIA roles -->
      <div
        class="rest-cloud"
        aria-label={t(unorganizedAriaLabelKey)}
        ondragover={(e) => e.preventDefault()}
        ondrop={handleRestDrop}
      >
        {#each restNumbers as n (n)}
          <button
            class="chip-pill"
            draggable="true"
            ondragstart={(e) => handleChipDragStart(n, e)}
            ondragend={() => (dragPayload = null)}
            onclick={() => handleChipClick(n)}
            title={t("trackLayout.restDragHint")}
          >
            {labelForRest(kind, n)}
          </button>
        {:else}
          <div class="muted small">{t("trackLayout.restEmpty")}</div>
        {/each}
      </div>
    </div>
  </div>
</div>

<style>
  .editor {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }
  .count-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .label {
    font-size: 0.8rem;
    color: var(--text-muted);
  }
  .input.narrow {
    width: 5rem;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: rgba(255, 255, 255, 0.02);
    color: var(--text);
    padding: 0.4rem 0.5rem;
  }
  .columns {
    display: flex;
    gap: 1rem;
  }
  .column {
    flex: 1;
    min-width: 0;
  }
  .subhead {
    font-weight: 600;
    margin-bottom: 0.25rem;
  }
  .muted.small {
    color: var(--text-muted);
    font-size: 0.75rem;
    margin-bottom: 0.5rem;
  }
  .order-list {
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 0.5rem;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    min-height: 3rem;
  }
  .list-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    background: rgba(255, 255, 255, 0.02);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 0.4rem 0.6rem;
    cursor: grab;
  }
  .chip {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    font-size: 0.85rem;
  }
  .chip .muted {
    color: var(--text-muted);
    font-size: 0.75rem;
  }
  .row-actions {
    display: flex;
    gap: 0.3rem;
  }
  .rest-cloud {
    border: 1px dashed var(--border);
    border-radius: var(--radius-sm);
    padding: 0.5rem;
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    align-content: flex-start;
    min-height: 3rem;
  }
  .chip-pill {
    border: 1px solid var(--border);
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.02);
    color: var(--text);
    padding: 0.3rem 0.8rem;
    font-size: 0.8rem;
    cursor: grab;
  }
  .btn {
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: rgba(255, 255, 255, 0.03);
    color: var(--text);
    padding: 0.3rem 0.6rem;
    cursor: pointer;
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .btn.small {
    font-size: 0.75rem;
    padding: 0.25rem 0.5rem;
  }
  .btn.danger {
    border-color: color-mix(in srgb, var(--danger) 50%, transparent);
    color: var(--danger);
  }
</style>
