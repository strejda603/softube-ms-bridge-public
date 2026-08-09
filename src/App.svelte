<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import * as ipc from "./lib/ipc";
  import { listLocales, setLocale } from "./lib/i18n.svelte";
  import { isKnownLocale } from "./lib/localeResolve";
  import type { BridgeConfigPatch, BridgeEvent, Lifecycle, RuntimeConfig, StatusSnapshot } from "./lib/ipc";
  import { parseModeFromLogLine } from "./lib/modeParse";
  import Topbar from "./lib/Topbar.svelte";
  import TabBar, { type TabId } from "./lib/TabBar.svelte";
  import ConnectionTab from "./lib/ConnectionTab.svelte";
  import LogDrawer from "./lib/LogDrawer.svelte";
  import TrackLayoutTab from "./lib/TrackLayoutTab.svelte";
  import type { Row } from "./lib/trackLayoutRows";
  import {
    clampInt,
    configBusOrderToRows,
    configInputOrderToRows,
    ensureRowsLength,
    flattenRowsToMax,
    rowsToConfigBusOrder,
    rowsToConfigInputOrder,
  } from "./lib/trackLayoutRows";
  import SendsColorsTab from "./lib/SendsColorsTab.svelte";
  import { normalizeC1SendMapping } from "./lib/sendsMapping";
  import PresetsTab from "./lib/PresetsTab.svelte";

  const INPUT_TOTAL_COUNT_STORAGE_KEY = "softubeMsBridge.inputTotalCount";
  const BUS_TOTAL_COUNT_STORAGE_KEY = "softubeMsBridge.busTotalCount";

  function safeLocalStorageGetInt(key: string): number | null {
    try {
      const raw = localStorage.getItem(key);
      const n = raw === null ? NaN : Number(raw);
      return Number.isFinite(n) ? n : null;
    } catch {
      return null;
    }
  }
  function safeLocalStorageSetTotal(key: string, value: number) {
    try {
      localStorage.setItem(key, String(value));
    } catch {
      // ignore
    }
  }

  let activeTab: TabId = $state("connection");
  let lifecycle = $state<Lifecycle>("standby");
  let error = $state(false);
  let mode: string | null = $state(null);
  let logLines: string[] = $state([]);
  let configLoaded = $state(false);

  let wsUrl = $state("ws://localhost:8080");
  let logJson = $state(false);
  let meteringIntervalMs = $state(100);
  let lastStartedKey = $state("");
  let applyInProgress = $state(false);
  let inputRows: Row[] = $state([]);
  let busRows: Row[] = $state([]);
  let inputTotalCount = $state(32);
  let busTotalCount = $state(16);
  let sends: number[] = $state([1, 2, 3, 4, 5, 6]);
  let mainColor = $state(0x00a5ff);
  let busColor = $state(0x800080);
  let lastLoadedKey = $state("");
  let lastPersistedKey = $state("");
  // Snapshot of canonicalKey() taken at the moment refreshFromConfig() actually fetched the
  // backend's config -- NOT re-derived later, because later re-derivation reads whatever the
  // form currently contains, which can have been mutated (e.g. by a preset load) in the async
  // gap before refreshStatus()'s own ipc.bridgeStatus() call resolves. See refreshStatus().
  let lastFetchedConfigKey = $state("");
  let statusSnapshot: StatusSnapshot | null = $state(null);
  let lastKnownWsUrl = $state("ws://localhost:8080");

  // Only ever READ when wsUrl is blank (see currentPatch() below) -- a blank write never needs
  // this effect to have flushed yet, only the preceding non-blank write does, and Svelte 5
  // flushes effects on a microtask between distinct native events, so a same-turn
  // clear-then-submit race is not reachable here.
  $effect(() => {
    const trimmed = wsUrl.trim();
    if (trimmed !== "") lastKnownWsUrl = trimmed;
  });

  function flushDraftSave() {
    if (!configLoaded || lifecycle !== "standby") return;
    const key = canonicalKey();
    if (key === lastPersistedKey) return;
    ipc.saveDraftConfig(currentPatch())
      .then(() => {
        lastPersistedKey = key;
      })
      .catch((e) => {
        appendLog(`[GUI] Failed to save draft config: ${e}`);
      });
  }

  $effect(() => {
    const key = canonicalKey(); // establishes the reactive dependency on every form field
    if (!configLoaded || lifecycle !== "standby" || key === lastPersistedKey) return;
    const timer = setTimeout(flushDraftSave, 800);
    return () => clearTimeout(timer);
  });

  function currentPatch(): Required<BridgeConfigPatch> {
    const trimmedWsUrl = wsUrl.trim();
    return {
      mixingStationWsUrl: trimmedWsUrl === "" ? lastKnownWsUrl : trimmedWsUrl,
      logJson,
      metering2IntervalMs: meteringIntervalMs,
      inputTrackOrder: rowsToConfigInputOrder(inputRows),
      busTrackOrder: rowsToConfigBusOrder(busRows),
      c1SendToMsBusNumber: sends,
      console1MainColor: mainColor,
      console1BusColor: busColor,
    };
  }

  function currentConfig(): RuntimeConfig {
    return currentPatch();
  }

  function canonicalKey(): string {
    return JSON.stringify(currentPatch());
  }

  const applyVisible = $derived(lifecycle === "running" && !!lastStartedKey && canonicalKey() !== lastStartedKey);
  const applyDisabled = $derived(applyInProgress);
  const liveConfig = $derived(currentConfig());
  const presetsDirty = $derived(canonicalKey() !== lastLoadedKey);

  function appendLog(line: string) {
    // Capped at 1000 lines: an unbounded array both grows the per-line copy cost here and
    // accumulates one permanent keyed DOM node per line in LogDrawer -- easily reached by
    // anyone enabling JSON debug logging at the metering interval's fast end (down to 30ms).
    logLines = [...logLines.slice(-999), line];
  }

  function handleConnectionChange(next: { wsUrl: string; logJson: boolean; meteringIntervalMs: number }) {
    wsUrl = next.wsUrl;
    logJson = next.logJson;
    meteringIntervalMs = next.meteringIntervalMs;
  }

  function handleTrackLayoutChange(next: {
    inputRows: Row[];
    busRows: Row[];
    inputTotalCount: number;
    busTotalCount: number;
  }) {
    inputRows = next.inputRows;
    busRows = next.busRows;
    inputTotalCount = next.inputTotalCount;
    busTotalCount = next.busTotalCount;
    sends = normalizeC1SendMapping(sends, busTotalCount);
    safeLocalStorageSetTotal(INPUT_TOTAL_COUNT_STORAGE_KEY, inputTotalCount);
    safeLocalStorageSetTotal(BUS_TOTAL_COUNT_STORAGE_KEY, busTotalCount);
  }

  function handleSendsChange(next: number[]) {
    sends = next;
  }

  function handleColorsChange(next: { mainColor: number; busColor: number }) {
    mainColor = next.mainColor;
    busColor = next.busColor;
  }

  function handleApplyLoadedConfig(loaded: RuntimeConfig) {
    applyConfigToForm(loaded);
    lastLoadedKey = canonicalKey();
  }

  function handleResetToDefault() {
    wsUrl = "ws://localhost:8080";
    logJson = false;
    meteringIntervalMs = 100;
    inputRows = [];
    busRows = [];
    sends = normalizeC1SendMapping([], busTotalCount);
    mainColor = 0x00a5ff;
    busColor = 0x800080;
    lastLoadedKey = canonicalKey();
  }

  /** Populates every form field from a full config -- shared by the live-config load path
   * (`refreshFromConfig`) and preset loading (`handleApplyLoadedConfig`), so both go through
   * the exact same dedupe/clamp logic already proven correct in Plans 5b/5c's review cycles. */
  function applyConfigToForm(config: RuntimeConfig) {
    wsUrl = config.mixingStationWsUrl;
    logJson = config.logJson;
    meteringIntervalMs = config.metering2IntervalMs;

    const inputRowsRaw = configInputOrderToRows(config.inputTrackOrder);
    const busRowsRaw = configBusOrderToRows(config.busTrackOrder);

    // There is no inputCount/busCount field on the wire -- total count is derived from the
    // highest configured channel/bus number, same as the old renderer's fallback when
    // cfg.inputCount is absent, but taken as the max against any total persisted locally
    // from a prior "Read from mixer"/manual edit so trimming the layout while offline
    // doesn't also shrink (and thus lose access to) the remembered full total.
    const storedInputTotal = safeLocalStorageGetInt(INPUT_TOTAL_COUNT_STORAGE_KEY);
    const storedBusTotal = safeLocalStorageGetInt(BUS_TOTAL_COUNT_STORAGE_KEY);
    inputTotalCount = clampInt(Math.max(flattenRowsToMax(inputRowsRaw), storedInputTotal ?? 0) || 32, 1, 512);
    busTotalCount = clampInt(Math.max(flattenRowsToMax(busRowsRaw), storedBusTotal ?? 0) || 16, 1, 512);

    sends = normalizeC1SendMapping(config.c1SendToMsBusNumber, busTotalCount);
    mainColor = config.console1MainColor;
    busColor = config.console1BusColor;

    // Dedupe/clamp against inputTotalCount/busTotalCount: a hand-edited or externally
    // authored config can carry duplicate or out-of-range entries (bridge-config's parser
    // skips individually-malformed entries but doesn't dedupe/clamp), and TrackOrderEditor's
    // keyed `{#each rows as row (row.join("-"))}` crashes on duplicate keys.
    inputRows = ensureRowsLength(inputRowsRaw, clampInt(inputRowsRaw.length, 0, inputTotalCount), inputTotalCount);
    busRows = ensureRowsLength(busRowsRaw, clampInt(busRowsRaw.length, 0, busTotalCount), busTotalCount);
  }

  async function refreshFromConfig() {
    try {
      const config = await ipc.getConfig();
      applyConfigToForm(config);
      const key = canonicalKey();
      lastLoadedKey = key;
      lastPersistedKey = key;
      lastFetchedConfigKey = key;
      configLoaded = true;
    } catch (e) {
      appendLog(`[GUI] Failed to read config: ${e}`);
    }
  }

  /** Bounded retry around the initial config read. Tauri's `bridge://ready` event fires (and
   * the webview's own `getConfig()` becomes callable) well before `onMount` can possibly run,
   * so a failed first attempt isn't a timing hiccup a later "ready" event could rescue -- it's
   * a real failure, and this is what actually gives it a few more chances before Start is left
   * permanently disabled behind the configLoaded gate. */
  async function refreshFromConfigWithRetry(attempts = 4, delayMs = 250) {
    for (let attempt = 0; attempt < attempts; attempt++) {
      await refreshFromConfig();
      if (configLoaded) return;
      if (attempt < attempts - 1) {
        await new Promise((resolve) => setTimeout(resolve, delayMs));
      }
    }
  }

  async function refreshStatus() {
    try {
      lifecycle = await ipc.bridgeStatus();
      if (lifecycle === "running") {
        if (mode === null) {
          mode = "Standard";
        }
        // Recovers the Apply-button dirty-check baseline on a GUI reload while the bridge is
        // already running. Seeded from lastFetchedConfigKey (a snapshot taken the moment
        // refreshFromConfig() actually fetched the backend's config), NOT from a fresh
        // canonicalKey() call here -- this function's own ipc.bridgeStatus() await is a real
        // async gap the user can act in (e.g. load a preset) before it resolves, and reading
        // live form state at that point would seed the wrong baseline. See lastFetchedConfigKey.
        if (configLoaded && !lastStartedKey) {
          lastStartedKey = lastFetchedConfigKey;
        }
      }
    } catch (e) {
      appendLog(`[GUI] Failed to read status: ${e}`);
    }
  }

  async function handleStart() {
    if (lifecycle === "running" || !configLoaded) return;
    try {
      await ipc.lifecycleStart(currentPatch());
      appendLog("[GUI] Bridge started");
      error = false;
      mode = "Standard";
      lastStartedKey = canonicalKey();
    } catch (e) {
      appendLog(`[GUI] Failed to start: ${e}`);
      error = true;
    }
  }

  async function handleStop() {
    if (lifecycle !== "running") return;
    try {
      await ipc.lifecycleStop();
      appendLog("[GUI] Bridge stopped");
    } catch (e) {
      appendLog(`[GUI] Failed to stop: ${e}`);
    } finally {
      error = false;
      mode = null;
      lastStartedKey = "";
    }
  }

  async function handleApply() {
    if (lifecycle !== "running" || applyInProgress) return;
    applyInProgress = true;
    try {
      await ipc.configApply(currentPatch());
      appendLog("[GUI] Changes applied");
      error = false;
      lastStartedKey = canonicalKey();
    } catch (e) {
      appendLog(`[GUI] Apply failed: ${e}`);
      error = true;
    } finally {
      applyInProgress = false;
    }
  }

  async function handleSupportKofi() {
    await ipc.openKofiPage();
  }

  /** Applies this launch's `--start`/`--stop`/`--preset`/`--ws`/`--interval`/`--log` CLI flags,
   * once, on mount. Order matters and mirrors the old Electron launcher: preset load happens
   * first, then ws/interval/log overrides are applied on top (so CLI overrides always win over
   * the preset's saved values), then start/stop. Requires configLoaded (the caller awaits
   * refreshFromConfigWithRetry() first) so handleStart()'s own configLoaded guard doesn't
   * silently swallow a `--start`. */
  async function applyLaunchArgs() {
    let launchArgs;
    try {
      launchArgs = await ipc.getLaunchArgs();
    } catch (e) {
      appendLog(`[GUI] Failed to read launch CLI args: ${e}`);
      return;
    }

    let presetLoadFailed = false;

    if (launchArgs.preset) {
      const wanted = launchArgs.preset.trim().toLowerCase();
      try {
        const presets = await ipc.listPresets();
        const match = presets.find((p) => p.name.trim().toLowerCase() === wanted);
        if (match) {
          const payload = await ipc.loadPreset(match.id);
          handleApplyLoadedConfig(payload.config);
        } else {
          appendLog(`[GUI] CLI: --preset "${launchArgs.preset}" not found; keeping current preset`);
        }
      } catch (e) {
        presetLoadFailed = true;
        appendLog(`[GUI] CLI: failed to load preset "${launchArgs.preset}": ${e}`);
      }
    }

    if (launchArgs.ws && launchArgs.ws.trim()) {
      const trimmed = launchArgs.ws.trim();
      wsUrl = /^wss?:\/\//i.test(trimmed) ? trimmed : `ws://${trimmed}`;
    }

    if (launchArgs.interval !== null) {
      meteringIntervalMs = Math.max(30, Math.min(1000, Math.trunc(launchArgs.interval)));
    }

    if (launchArgs.log) {
      logJson = true;
    }

    if (launchArgs.start && !presetLoadFailed) {
      await handleStart();
    } else if (launchArgs.stop) {
      await handleStop();
    }
  }

  onMount(() => {
    let disposed = false;
    let unlistenReady: (() => void) | undefined;
    let unlistenEvent: (() => void) | undefined;
    let unlistenStatus: (() => void) | undefined;
    let unlistenClose: (() => void) | undefined;

    (async () => {
      try {
        const launchLang = await ipc.getLaunchLang();
        if (launchLang && isKnownLocale(launchLang, listLocales().map((locale) => locale.code))) {
          setLocale(launchLang, /* persist */ false);
        }
      } catch (e) {
        appendLog(`[GUI] Failed to read launch language override: ${e}`);
      }

      await refreshFromConfigWithRetry();
      await applyLaunchArgs();
      await refreshStatus();

      const readyUnlisten = await ipc.onReady(() => {
        refreshStatus();
      });
      if (disposed) {
        readyUnlisten();
      } else {
        unlistenReady = readyUnlisten;
      }

      const eventUnlisten = await ipc.onBridgeEvent((event: BridgeEvent) => {
        if (event.type === "log") {
          appendLog(event.data);
          const parsedMode = parseModeFromLogLine(event.data);
          if (parsedMode) mode = parsedMode;
        } else if (event.type === "lifecycleChanged") {
          lifecycle = event.data;
        } else if (event.type === "crashed") {
          appendLog(`[GUI] Bridge crashed: ${event.data}`);
          error = true;
        }
      });
      if (disposed) {
        eventUnlisten();
      } else {
        unlistenEvent = eventUnlisten;
      }

      const statusUnlisten = await ipc.onStatusUpdate((snapshot) => {
        statusSnapshot = snapshot;
      });
      if (disposed) {
        statusUnlisten();
      } else {
        unlistenStatus = statusUnlisten;
      }

      try {
        const seed = await ipc.getStatus();
        if (seed && statusSnapshot === null) statusSnapshot = seed;
      } catch (e) {
        appendLog(`[GUI] Failed to read status snapshot: ${e}`);
      }

      const closeUnlisten = await getCurrentWindow().onCloseRequested((event) => {
        // Rust's on_window_event already owns close -> shutdown -> destroy -> exit via
        // prevent_close() -- this stops the JS wrapper's own unconditional destroy() from
        // racing that sequence and cutting the real hardware cleanup short. This does NOT
        // block the actual quit; Rust drives that independently.
        event.preventDefault();
        flushDraftSave();
      });
      if (disposed) {
        closeUnlisten();
      } else {
        unlistenClose = closeUnlisten;
      }
    })();

    return () => {
      disposed = true;
      unlistenReady?.();
      unlistenEvent?.();
      unlistenStatus?.();
      unlistenClose?.();
    };
  });
</script>

<div class="app">
  <Topbar
    {lifecycle}
    {error}
    {mode}
    {applyVisible}
    {applyDisabled}
    {configLoaded}
    {statusSnapshot}
    onStart={handleStart}
    onStop={handleStop}
    onApply={handleApply}
    onSupportKofi={handleSupportKofi}
  />
  <TabBar active={activeTab} onSelect={(id) => (activeTab = id)} />
  <div class="tab-content">
    {#if activeTab === "connection"}
      <ConnectionTab {wsUrl} {logJson} {meteringIntervalMs} onChange={handleConnectionChange} />
    {:else if activeTab === "trackLayout"}
      <TrackLayoutTab
        {inputRows}
        {busRows}
        {inputTotalCount}
        {busTotalCount}
        {wsUrl}
        onChange={handleTrackLayoutChange}
      />
    {:else if activeTab === "sendsColors"}
      <SendsColorsTab
        {sends}
        {busTotalCount}
        {mainColor}
        {busColor}
        onSendsChange={handleSendsChange}
        onColorsChange={handleColorsChange}
      />
    {:else if activeTab === "presets"}
      <PresetsTab
        config={liveConfig}
        isDirty={presetsDirty}
        onApplyLoadedConfig={handleApplyLoadedConfig}
        onResetToDefault={handleResetToDefault}
      />
    {/if}
  </div>
  <LogDrawer lines={logLines} />
</div>

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  .tab-content {
    flex: 1;
    overflow-y: auto;
  }
</style>
