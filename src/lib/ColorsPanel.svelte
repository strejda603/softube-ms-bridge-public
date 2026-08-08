<script lang="ts">
  import { t } from "./i18n.svelte";
  import { formatColorIntHex, parseHexInt, rgbToSoftubeInt, softubeIntToRgb } from "./colorUtils";

  let {
    mainColor,
    busColor,
    onColorsChange,
  }: {
    mainColor: number;
    busColor: number;
    onColorsChange: (next: { mainColor: number; busColor: number }) => void;
  } = $props();

  function handlePickerInput(kind: "main" | "bus", hex: string) {
    const intVal = rgbToSoftubeInt(hex);
    if (intVal === null) return;
    onColorsChange(kind === "main" ? { mainColor: intVal, busColor } : { mainColor, busColor: intVal });
  }

  function handleTextChange(kind: "main" | "bus", raw: string) {
    const intVal = parseHexInt(raw);
    if (intVal === null) return;
    onColorsChange(kind === "main" ? { mainColor: intVal, busColor } : { mainColor, busColor: intVal });
  }
</script>

<div class="panel">
  <div class="subhead">{t("colors.title")}</div>
  <div class="color-grid">
    <div class="color-row">
      <span class="label">{t("colors.main")}</span>
      <input
        type="color"
        class="picker"
        aria-label={t("colors.pickerAriaLabel", { color: t("colors.main") })}
        value={softubeIntToRgb(mainColor)}
        oninput={(e) => handlePickerInput("main", (e.target as HTMLInputElement).value)}
      />
      <input
        class="input"
        aria-label={t("colors.hexAriaLabel", { color: t("colors.main") })}
        value={formatColorIntHex(mainColor)}
        onchange={(e) => handleTextChange("main", (e.target as HTMLInputElement).value)}
      />
    </div>
    <div class="color-row">
      <span class="label">{t("colors.bus")}</span>
      <input
        type="color"
        class="picker"
        aria-label={t("colors.pickerAriaLabel", { color: t("colors.bus") })}
        value={softubeIntToRgb(busColor)}
        oninput={(e) => handlePickerInput("bus", (e.target as HTMLInputElement).value)}
      />
      <input
        class="input"
        aria-label={t("colors.hexAriaLabel", { color: t("colors.bus") })}
        value={formatColorIntHex(busColor)}
        onchange={(e) => handleTextChange("bus", (e.target as HTMLInputElement).value)}
      />
    </div>
  </div>
</div>

<style>
  .panel {
    padding: 1.5rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }
  .subhead {
    font-weight: 600;
  }
  .color-grid {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    max-width: 20rem;
  }
  .color-row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }
  .label {
    width: 4rem;
    font-size: 0.85rem;
    color: var(--text-muted);
  }
  .picker {
    width: 2.25rem;
    height: 2.25rem;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: transparent;
    cursor: pointer;
  }
  .input {
    flex: 1;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: rgba(255, 255, 255, 0.02);
    color: var(--text);
    padding: 0.4rem 0.6rem;
    font-family: var(--mono);
  }
</style>
