<script lang="ts">
  import { onMount } from "svelte";
  import { getVersion } from "@tauri-apps/api/app";
  import { getLocale, listLocales, setLocale, t } from "./i18n.svelte";
  import * as ipc from "./ipc";
  import type { Lifecycle, StatusSnapshot, UpdateCheckResult } from "./ipc";
  import brandIcon from "../assets/icon.png";
  import kofiIcon from "../assets/logomarkLogo.png";

  let {
    lifecycle,
    error,
    mode,
    applyVisible,
    applyDisabled,
    configLoaded,
    statusSnapshot,
    onStart,
    onStop,
    onApply,
    onSupportKofi,
  }: {
    lifecycle: Lifecycle;
    error: boolean;
    mode: string | null;
    applyVisible: boolean;
    applyDisabled: boolean;
    configLoaded: boolean;
    statusSnapshot: StatusSnapshot | null;
    onStart: () => void;
    onStop: () => void;
    onApply: () => void;
    onSupportKofi: () => void;
  } = $props();

  let showSettings = $state(false);
  let appVersion: string | null = $state(null);
  let settingsRef: HTMLDivElement | undefined = $state();
  let lastUpdateInfo: UpdateCheckResult | null = $state(null);
  let updateDismissedThisSession = $state(false);
  let updateStatusMessage: string | null = $state(null);
  let updateStatusTimer: ReturnType<typeof setTimeout> | undefined;

  /** Silent on-launch check -- any failure is swallowed, matching the original's "no user-visible
   * feedback for the automatic check" behavior; only the manual button surfaces errors. */
  async function checkForUpdateSilently() {
    try {
      const result = await ipc.checkForUpdate();
      if (result.available) {
        lastUpdateInfo = result;
      }
    } catch {
      // Swallowed -- see doc comment above.
    }
  }

  /** Shows a transient status message, auto-clearing after 4 seconds -- matches the original's
   * behavior for the manual "Check for Updates" button's up-to-date/error feedback. */
  function showUpdateStatusMessage(key: string) {
    updateStatusMessage = t(key);
    clearTimeout(updateStatusTimer);
    updateStatusTimer = setTimeout(() => {
      updateStatusMessage = null;
    }, 4000);
  }

  async function handleCheckForUpdate() {
    updateDismissedThisSession = false;
    try {
      const result = await ipc.checkForUpdate();
      if (result.available) {
        lastUpdateInfo = result;
        return;
      }
      showUpdateStatusMessage(result.error ? "update.checkFailed" : "update.upToDate");
    } catch {
      showUpdateStatusMessage("update.checkFailed");
    }
  }

  function handleDownload() {
    if (!lastUpdateInfo) return;
    void ipc.openDownload(lastUpdateInfo.downloadUrl ?? lastUpdateInfo.releaseUrl ?? "");
  }

  function handleDismissUpdate() {
    updateDismissedThisSession = true;
  }

  onMount(() => {
    getVersion()
      .then((v) => {
        appVersion = v;
      })
      .catch(() => {
        // Inert display-only info -- leave appVersion null, the popover simply omits the line.
      });
    checkForUpdateSilently();
  });

  // Closes the popover on an outside click or Escape. Only listens while open, matching this
  // codebase's existing $effect-based lifecycle pattern (e.g. App.svelte's draft-save timer).
  $effect(() => {
    if (!showSettings) return;
    function handlePointerDown(e: MouseEvent) {
      if (settingsRef && !settingsRef.contains(e.target as Node)) {
        showSettings = false;
      }
    }
    function handleKeydown(e: KeyboardEvent) {
      if (e.key === "Escape") showSettings = false;
    }
    window.addEventListener("mousedown", handlePointerDown);
    window.addEventListener("keydown", handleKeydown);
    return () => {
      window.removeEventListener("mousedown", handlePointerDown);
      window.removeEventListener("keydown", handleKeydown);
    };
  });

  const STATUS_DOTS: {
    key: keyof StatusSnapshot;
    kind: "midi" | "process";
    labelKey: string;
    fullKey: string;
  }[] = [
    {
      key: "mixingStation",
      kind: "process",
      labelKey: "status.mixingStation",
      fullKey: "status.mixingStationFull",
    },
    {
      key: "console1Osd",
      kind: "process",
      labelKey: "status.console1Osd",
      fullKey: "status.console1OsdFull",
    },
  ];

  // `statusSnapshot === null` means "no snapshot received yet" -- coercing null to false would
  // flash both dots red on every launch, even on integrations that ARE actually present, so
  // it must render as its own "pending" state.
  function dotState(key: keyof StatusSnapshot): "pending" | "on" | "off" {
    if (!statusSnapshot) return "pending";
    return statusSnapshot[key] ? "on" : "off";
  }

  function dotAriaLabel(
    kind: "midi" | "process",
    fullKey: string,
    state: "pending" | "on" | "off",
  ): string {
    const stateKey =
      state === "pending"
        ? "status.pending"
        : kind === "midi"
          ? state === "on"
            ? "status.connected"
            : "status.notConnected"
          : state === "on"
            ? "status.running"
            : "status.notRunning";
    return t("status.ariaLabel", { name: t(fullKey), state: t(stateKey) });
  }

  // Computed once per render, not once per template read -- keeps a dot's visual state and its
  // aria-label consistent by construction.
  const dots = $derived(
    STATUS_DOTS.map((dot) => {
      const state = dotState(dot.key);
      return {
        ...dot,
        state,
        ariaLabel: dotAriaLabel(dot.kind, dot.fullKey, state),
      };
    }),
  );

  const isSends = $derived(/^Sends\b/i.test(mode ?? ""));
  const statusLabel = $derived(
    error ? t("topbar.error") : lifecycle === "running" ? t("topbar.running") : t("topbar.stopped"),
  );
</script>

<header class="topbar">
  <div class="brand">
    <img src={brandIcon} alt="" class="brand-icon" />
    <div class="brand-text">
      <span class="title">{t("app.title")}</span>
      <span class="subtitle">{t("app.subtitle")}</span>
    </div>
  </div>
  <button class="btn kofi" title={t("kofi.title")} onclick={onSupportKofi}>
    <img src={kofiIcon} alt="Ko-fi tips" class="kofiimg" style="object-fit: contain;" aria-hidden="true">
    {t("kofi.button")}
  </button>
  <div class="actions">
    <div class="status-indicators" role="group" aria-label={t("status.groupAriaLabel")}>
      {#each dots as dot (dot.key)}
        <span class="status-item" title={t(dot.fullKey)}>
          <span class="status-dot" role="img" aria-label={dot.ariaLabel} data-state={dot.state}
          ></span>
          <span class="status-label">{t(dot.labelKey)}</span>
        </span>
      {/each}
    </div>
    <div class="badge mode" class:sends={isSends}>
      {t("topbar.mode", { mode: mode ?? t("topbar.modeUnset") })}
    </div>
    <div class="badge status" class:error class:running={lifecycle === "running" && !error}>
      {statusLabel}
    </div>
    {#if applyVisible}
      <button
        class="btn primary"
        title={t("topbar.applyChangesTitle")}
        disabled={applyDisabled}
        onclick={onApply}
      >
        {t("topbar.applyChanges")}
      </button>
    {/if}
    <button
      class="btn primary"
      disabled={lifecycle === "running" || !configLoaded}
      title={!configLoaded ? t("topbar.startDisabledTitle") : undefined}
      onclick={onStart}
    >
      {t("topbar.start")}
    </button>
    <button class="btn" disabled={lifecycle !== "running"} onclick={onStop}>
      {t("topbar.stop")}
    </button>
    <div class="settings" bind:this={settingsRef}>
      <button
        class="btn icon-btn"
        aria-label={t("footer.languageAriaLabel")}
        aria-expanded={showSettings}
        onclick={() => (showSettings = !showSettings)}
      >
        ⚙
      </button>
      {#if showSettings}
        <div class="popover">
          <label class="popover-row" for="languageSelect">{t("footer.language")}</label>
          <select
            id="languageSelect"
            value={getLocale()}
            onchange={(e) => setLocale((e.target as HTMLSelectElement).value)}
          >
            {#each listLocales() as locale (locale.code)}
              <option value={locale.code}>{locale.name}</option>
            {/each}
          </select>
          {#if appVersion}
            <div class="popover-version">{t("footer.version", { version: appVersion })}</div>
          {/if}
          <div class="popover-row">
            <button class="btn" onclick={handleCheckForUpdate}>{t("update.checkButton")}</button>
          </div>
          {#if updateStatusMessage}
            <div class="popover-update-status">{updateStatusMessage}</div>
          {/if}
          {#if lastUpdateInfo?.available && !updateDismissedThisSession}
            <div class="popover-update-pill">
              <span>{t("update.available", { version: lastUpdateInfo.latestVersion ?? "" })}</span>
              <button class="btn primary" onclick={handleDownload}>{t("update.download")}</button>
              <button
                class="btn icon-btn"
                aria-label={t("update.dismiss")}
                onclick={handleDismissUpdate}
              >
                ×
              </button>
            </div>
          {/if}
        </div>
      {/if}
    </div>
  </div>
</header>

<style>
  .topbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 1rem;
    height: 56px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 0.6rem;
  }
  .brand-text {
    display: flex;
    flex-direction: column;
    justify-content: center;
    line-height: 1.25;
  }
  .title {
    font-weight: 600;
  }
  .subtitle {
    font-size: 0.7rem;
    color: var(--text-muted);
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }
  .status-indicators {
    display: flex;
    gap: 0.5rem;
  }
  .status-item {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    font-size: 0.75rem;
    color: var(--text-muted);
  }
  .status-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    box-sizing: border-box;
  }
  .status-dot[data-state="pending"] {
    background: var(--text-muted);
  }
  .status-dot[data-state="on"] {
    background: var(--success);
  }
  .status-dot[data-state="off"] {
    background: transparent;
    border: 1.5px solid var(--danger);
  }
  .badge {
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 0.25rem 0.75rem;
    font-size: 0.8rem;
    background: rgba(255, 255, 255, 0.02);
  }
  .badge.mode.sends {
    border-color: rgba(59, 130, 246, 0.6);
    background: var(--primary-bg);
  }
  .badge.status.running {
    border-color: color-mix(in srgb, var(--success) 50%, transparent);
    background: color-mix(in srgb, var(--success) 10%, transparent);
  }
  .badge.status.error {
    border-color: color-mix(in srgb, var(--danger) 50%, transparent);
    background: color-mix(in srgb, var(--danger) 10%, transparent);
    color: var(--danger);
  }
  .btn {
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: rgba(255, 255, 255, 0.03);
    color: var(--text);
    padding: 0.4rem 0.9rem;
    cursor: pointer;
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .btn.primary {
    background: var(--primary);
    border-color: var(--primary);
    color: white;
  }
  .btn.primary:disabled {
    background: rgba(59, 130, 246, 0.3);
    border-color: transparent;
  }
  .btn.kofi {
    background: #72a4f2;
    border-color: #72a4f2;
    color: white;
    box-shadow: 1px 1px 0px rgba(0, 0, 0, 0.2);
  }
  .btn.kofi:hover {
    opacity: 0.85;
    color: #f5f5f5 !important;
  }

  .kofiimg {
    display: initial !important;
    vertical-align: middle;
    width: 19px !important;
    height: 15px !important;
    padding-top: 0 !important;
    padding-bottom: 0 !important;
    border: none;
    margin-top: 0 !important;
    margin-bottom: 0 !important;
    margin-left: 0 !important;
    margin-right: 5px !important;
    animation: kofi-wiggle 3s infinite;
  }

  .brand-icon {
    width: 28px;
    height: 28px;
    border-radius: 4px;
    flex-shrink: 0;
  }
  .settings {
    position: relative;
  }
  .icon-btn {
    font-size: 0.95rem;
    line-height: 1;
    padding: 0.4rem 0.6rem;
  }
  .popover {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    background: var(--panel-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 0.75rem;
    min-width: 160px;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    z-index: 20;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4);
  }
  .popover-row {
    font-size: 0.75rem;
    color: var(--text-muted);
  }
  .popover select {
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--text);
    padding: 0.3rem 0.5rem;
  }
  .popover-version {
    font-size: 0.7rem;
    color: var(--text-muted);
    border-top: 1px solid var(--border);
    padding-top: 0.5rem;
  }
  .popover-update-status {
    font-size: 0.7rem;
    color: var(--text-muted);
  }
  .popover-update-pill {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.75rem;
    flex-wrap: wrap;
  }

  @keyframes kofi-wiggle {
    0% {
      transform: rotate(0) scale(1);
    }
    60% {
      transform: rotate(0) scale(1);
    }
    75% {
      transform: rotate(0) scale(1.12);
    }
    80% {
      transform: rotate(0) scale(1.1);
    }
    84% {
      transform: rotate(-10deg) scale(1.1);
    }
    88% {
      transform: rotate(10deg) scale(1.1);
    }
    92% {
      transform: rotate(-10deg) scale(1.1);
    }
    96% {
      transform: rotate(10deg) scale(1.1);
    }
    100% {
      transform: rotate(0) scale(1);
    }
  }
</style>
