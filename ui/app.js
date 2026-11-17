/* Capto — settings window.
 *
 * Plain DOM, no framework. The window renders a snapshot produced by Rust and
 * sends the whole settings object back on save; every operation that can block
 * happens on a worker thread inside the shell.
 */
(() => {
  "use strict";

  const tauri = window.__TAURI__;
  if (!tauri) {
    document.getElementById("offline").hidden = false;
    return;
  }
  const { invoke } = tauri.core;
  const { listen } = tauri.event;

  const $ = (id) => document.getElementById(id);
  const ui = {
    pill: $("status-pill"),
    subtitle: $("mode-subtitle"),
    banners: $("banners"),
    summary: $("target-summary"),
    badge: $("mode-badge"),
    problem: $("target-problem"),
    capture: $("capture-now"),
    snipNow: $("snip-now"),
    selectRegion: $("select-region"),
    selectWindow: $("select-window"),
    chooseWindow: $("choose-window"),
    lastCapture: $("last-capture"),
    lastFilename: $("last-filename"),
    lastMeta: $("last-meta"),
    openFolderInline: $("open-folder-inline"),
    modes: $("modes"),
    regionDetail: $("region-detail"),
    windowDetail: $("window-detail"),
    outputDir: $("output-dir"),
    browse: $("browse"),
    openFolder: $("open-folder"),
    format: $("format"),
    qualityField: $("quality-field"),
    quality: $("quality"),
    qualityValue: $("quality-value"),
    pattern: $("pattern"),
    patternPreview: $("pattern-preview"),
    patternWarning: $("pattern-warning"),
    shortcutDisplay: $("shortcut-display"),
    recordShortcut: $("record-shortcut"),
    clearShortcut: $("clear-shortcut"),
    shortcutHelp: $("shortcut-help"),
    autostart: $("autostart"),
    showToast: $("show-toast"),
    factBackend: $("fact-backend"),
    factDisplay: $("fact-display"),
    factScreen: $("fact-screen"),
    factConfig: $("fact-config"),
    notes: $("backend-notes"),
    saveState: $("save-state"),
    saveStateText: $("save-state-text"),
    version: $("version"),
    hide: $("hide"),
    quit: $("quit"),
    modal: $("window-modal"),
    windowList: $("window-list"),
  };

  const MODE_LABELS = {
    region: "Remembered Region",
    select_area: "Immediate Area",
    full_screen: "Full Screen",
    pinned_window: "Pinned Window",
    foreground_window: "Foreground",
  };

  const KEY_PATTERN =
    /^(Key[A-Z]|Digit[0-9]|F([1-9]|1[0-9]|2[0-4])|Space|Enter|Tab|Insert|Home|End|PageUp|PageDown|Arrow(Up|Down|Left|Right)|PrintScreen|Minus|Equal|Comma|Period|Slash|Semicolon|Quote|BracketLeft|BracketRight|Backslash|Backquote)$/;

  let snapshot = null;
  let form = null;
  let dirty = false;
  let recording = false;
  let busyDepth = 0;
  let transient = null;
  const dismissed = new Set();

  const clone = (value) => JSON.parse(JSON.stringify(value));

  // -------------------------------------------------------------------------
  // IPC helpers
  // -------------------------------------------------------------------------

  async function guard(work) {
    busyDepth += 1;
    document.body.classList.add("busy");
    try {
      return await work();
    } catch (error) {
      showError(error);
      return undefined;
    } finally {
      busyDepth -= 1;
      if (busyDepth === 0) document.body.classList.remove("busy");
    }
  }

  function showError(error) {
    const message =
      error && typeof error === "object" && error.message
        ? error.message
        : String(error);
    const hint = error && typeof error === "object" && error.hint ? error.hint : "";
    transient = {
      key: "transient",
      tone: "error",
      title: "Action failed",
      text: hint ? `${message} ${hint}` : message,
    };
    renderBanners();
    if (ui.saveStateText) ui.saveStateText.textContent = "Error";
  }

  // -------------------------------------------------------------------------
  // Snapshot rendering
  // -------------------------------------------------------------------------

  function applySnapshot(next, { adopt = false } = {}) {
    if (!next) return;
    snapshot = next;

    if (form === null || adopt) {
      form = clone(next.settings);
      fillForm();
      dirty = false;
    } else {
      // Region and window are chosen outside the form; keep them current even
      // when the user has unsaved edits in the text fields.
      form.region = next.settings.region;
      form.window = next.settings.window;
    }
    render();
  }

  function fillForm() {
    if (!form) return;
    ui.outputDir.value = form.output_dir || "";
    ui.pattern.value = form.filename_pattern || "";
    ui.quality.value = String(form.jpeg_quality || 90);
    ui.autostart.checked = Boolean(form.launch_at_login);
    ui.showToast.checked = Boolean(form.show_toast);
    for (const radio of document.querySelectorAll('input[name="mode"]')) {
      radio.checked = radio.value === form.mode;
    }
    renderShortcut();
    renderFormat();
    renderQuality();
    renderPreview();
  }

  function render() {
    if (!snapshot || !form) return;
    renderChrome();
    renderBanners();
    renderTarget();
    renderModes();
    renderShortcut();
    renderFormat();
    renderQuality();
    renderPreview();
    renderEnvironment();
    ui.version.textContent = `v${snapshot.version}`;
    if (!dirty && ui.saveStateText && !ui.saveStateText.textContent) {
      ui.saveStateText.textContent = "Saved";
    }
  }

  function renderChrome() {
    const status = snapshot.status;
    ui.pill.classList.remove("ready", "paused", "error");
    if (snapshot.startup_error || !status) {
      ui.pill.textContent = "Unavailable";
      ui.pill.classList.add("error");
      ui.subtitle.textContent = "Capture backend is not available";
    } else if (status.paused) {
      ui.pill.textContent = "Paused";
      ui.pill.classList.add("paused");
      ui.subtitle.textContent = `Paused · ${MODE_LABELS[form.mode]} target`;
    } else {
      ui.pill.textContent = "Ready";
      ui.pill.classList.add("ready");
      ui.subtitle.textContent = `${MODE_LABELS[form.mode]} target · ${
        form.shortcut ? "shortcut active" : "tray only"
      }`;
    }
  }

  function renderBanners() {
    const banners = [];
    if (transient) banners.push(transient);
    if (snapshot.startup_error) {
      const error = snapshot.startup_error;
      banners.push({
        key: "startup",
        tone: "error",
        title: "Capturing is not available",
        text: error.hint ? `${error.message} ${error.hint}` : error.message,
      });
    }
    if (snapshot.load_warning) {
      banners.push({
        key: "load",
        tone: "warn",
        title: "Settings",
        text: snapshot.load_warning,
      });
    }
    if (snapshot.shortcut_error) {
      banners.push({
        key: "shortcut",
        tone: "warn",
        title: "Global shortcut",
        text: snapshot.shortcut_error,
      });
    }
    if (snapshot.autostart_error) {
      banners.push({
        key: "autostart",
        tone: "warn",
        title: "Start at login",
        text: snapshot.autostart_error,
      });
    }

    ui.banners.replaceChildren();
    for (const banner of banners) {
      if (banner.key !== "transient" && dismissed.has(banner.key)) continue;
      const node = document.createElement("div");
      node.className = `banner ${banner.tone}`;
      const text = document.createElement("div");
      text.className = "banner-text";
      const title = document.createElement("strong");
      title.textContent = banner.title;
      text.append(title, document.createTextNode(banner.text));
      const close = document.createElement("button");
      close.type = "button";
      close.textContent = "×";
      close.setAttribute("aria-label", "Dismiss");
      close.addEventListener("click", () => {
        dismissed.add(banner.key);
        if (banner.key === "transient") transient = null;
        renderBanners();
      });
      node.append(text, close);
      ui.banners.append(node);
    }
  }

  function renderTarget() {
    const status = snapshot.status;
    const ready = Boolean(status && status.target_ready && !status.paused);

    ui.summary.textContent = status
      ? status.capture_summary
      : "Capturing is not available on this system.";

    ui.badge.textContent = MODE_LABELS[form.mode] || form.mode;
    ui.badge.className = "badge";
    if (form.mode === "pinned_window") ui.badge.classList.add("pinned");
    if (form.mode === "foreground_window") ui.badge.classList.add("foreground");
    if (form.mode === "select_area") ui.badge.classList.add("select-area");
    if (form.mode === "region") ui.badge.classList.add("region");
    if (form.mode === "full_screen") ui.badge.classList.add("full-screen");

    let problem = null;
    if (status && !status.target_ready && status.target_problem) {
      problem = status.target_problem;
    } else if (status && status.paused) {
      problem = "Capturing is paused. Use the tray menu or the pause action to resume.";
    }
    ui.problem.hidden = !problem;
    ui.problem.textContent = problem || "";

    ui.capture.disabled = !ready || !status;
    if (ui.snipNow) {
      ui.snipNow.disabled =
        !status || !status.capabilities.interactive_region_selection || Boolean(status.paused);
    }
    ui.selectRegion.disabled = !status || !status.capabilities.interactive_region_selection;
    ui.selectWindow.disabled =
      !status || !status.capabilities.interactive_window_selection;
    ui.chooseWindow.disabled = !status || !status.capabilities.window_list;

    ui.selectRegion.hidden = form.mode !== "region";
    ui.selectWindow.hidden = form.mode !== "pinned_window";
    ui.chooseWindow.hidden = form.mode !== "pinned_window";

    const last = status && status.last_capture;
    ui.lastCapture.hidden = !last;
    if (last) {
      ui.lastFilename.textContent = `${last.filename}${
        last.warning ? " · note" : ""
      }`;
      ui.lastFilename.title = last.path;
      ui.lastMeta.textContent = `${last.width}×${last.height} · ${Math.max(
        1,
        Math.round(last.bytes / 1024)
      )} KB · ${last.captured_at} · ${last.target_label}${
        last.warning ? ` · ${last.warning}` : ""
      }`;
      ui.openFolderInline.disabled = !status;
    } else {
      ui.openFolderInline.disabled = true;
    }
  }

  function renderModes() {
    const caps = (snapshot.status && snapshot.status.capabilities) || {};
    const availability = {
      select_area: caps.interactive_region_selection !== false && caps.region !== false,
      region: caps.region !== false,
      full_screen: caps.full_screen !== false,
      pinned_window: caps.pinned_window !== false,
      foreground_window: Boolean(caps.foreground_window),
    };
    for (const label of ui.modes.querySelectorAll(".mode-chip")) {
      const mode = label.dataset.mode;
      const radio = label.querySelector("input");
      radio.checked = form.mode === mode;
      radio.disabled = !availability[mode];
      label.title = availability[mode]
        ? ""
        : "This mode is not supported by the current capture backend.";
    }

    const region = form.region;
    ui.regionDetail.textContent =
      region && region.width >= 5
        ? `${region.width}×${region.height}`
        : "No area set";

    ui.windowDetail.textContent = form.window
      ? form.window.title || form.window.class || `window ${form.window.window_id}`
      : "No window pinned";
  }

  function renderShortcut() {
    if (!form) return;
    const value = (form.shortcut || "").trim();
    ui.shortcutDisplay.textContent = recording
      ? "Press a key combination…"
      : value || "No global shortcut";
    ui.shortcutDisplay.classList.toggle("recording", recording);
    ui.shortcutDisplay.classList.toggle("empty", !value && !recording);
    ui.clearShortcut.disabled = !value || recording;
    ui.recordShortcut.textContent = recording ? "Listening…" : "Change…";
    ui.shortcutHelp.textContent = recording
      ? "Press a combination such as Ctrl+Alt+S. Escape cancels, Backspace clears."
      : value
        ? "Click “Change”, then press a combination such as Ctrl+Alt+S. The tray menu always works."
        : "No global shortcut. Use “Change” to add one; the tray menu keeps working regardless.";
  }

  function renderFormat() {
    for (const button of ui.format.querySelectorAll("button")) {
      button.classList.toggle("active", button.dataset.format === form.format);
    }
    ui.qualityField.hidden = form.format !== "jpeg";
  }

  function renderQuality() {
    const value = Number(ui.quality.value) || 90;
    ui.qualityValue.textContent = String(value);
    ui.quality.style.setProperty("--fill", `${value}%`);
  }

  function renderPreview() {
    ui.patternPreview.textContent = previewName();
    const hasToken = /\{(date|time|datetime)\}/.test(form.filename_pattern || "");
    ui.patternWarning.hidden = hasToken;
    ui.patternWarning.textContent = hasToken
      ? ""
      : "This pattern has no {date}/{time} token. Names stay unique through numbered suffixes.";
  }

  function previewName() {
    const now = new Date();
    const pad = (value) => String(value).padStart(2, "0");
    const date = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
    const time = `${pad(now.getHours())}-${pad(now.getMinutes())}-${pad(
      now.getSeconds()
    )}`;
    const mode = form.mode === "region" ? "region" : "window";
    const base = (form.filename_pattern || "")
      .replaceAll("{date}", date)
      .replaceAll("{time}", time)
      .replaceAll("{datetime}", `${date}_${time}`)
      .replaceAll("{mode}", mode);
    return `${base}.${form.format === "jpeg" ? "jpg" : "png"}`;
  }

  function renderEnvironment() {
    const status = snapshot.status;
    ui.factBackend.textContent = status ? status.backend : "Not available";
    ui.factDisplay.textContent = status ? status.display_server : "—";
    ui.factScreen.textContent =
      status && status.screen
        ? `${status.screen.width}×${status.screen.height} at ${status.screen.x},${status.screen.y}`
        : "—";
    ui.factConfig.textContent = snapshot.config_path;
    ui.factConfig.title = snapshot.config_path;

    ui.notes.replaceChildren();
    for (const note of (status && status.notes) || []) {
      const item = document.createElement("li");
      item.textContent = note;
      ui.notes.append(item);
    }
  }

  // -------------------------------------------------------------------------
  // Actions & Auto-Save
  // -------------------------------------------------------------------------

  let saveTimeout = null;
  function scheduleAutoSave(delay = 350) {
    if (ui.saveState) {
      ui.saveState.classList.add("saving");
    }
    if (ui.saveStateText) {
      ui.saveStateText.textContent = "Saving…";
    }
    if (saveTimeout) clearTimeout(saveTimeout);
    saveTimeout = setTimeout(async () => {
      await save({ quiet: true });
      if (ui.saveState) {
        ui.saveState.classList.remove("saving");
      }
      if (ui.saveStateText) {
        ui.saveStateText.textContent = "Saved";
      }
    }, delay);
  }

  function markDirty() {
    scheduleAutoSave(350);
  }

  async function refresh({ adopt = false } = {}) {
    if (busyDepth > 0) return;
    const next = await guard(() => invoke("get_state"));
    if (next) applySnapshot(next, { adopt });
  }

  async function save({ quiet = false } = {}) {
    if (!form) return false;
    form.version = 1;
    const next = await guard(() => invoke("save_settings", { settings: form }));
    if (!next) return false;
    transient = null;
    snapshot = next;
    form = clone(next.settings);
    fillForm();
    dirty = false;
    render();
    if (ui.saveStateText) {
      ui.saveStateText.textContent = next.shortcut_error
        ? "Saved (shortcut unchanged)"
        : "Saved";
    }
    return true;
  }

  async function captureNow() {
    stopRecording();
    transient = null;
    const outcome = await guard(() => invoke("capture_now"));
    if (outcome && ui.saveStateText) ui.saveStateText.textContent = "Screenshot saved";
    await refresh();
  }

  async function captureSnip() {
    stopRecording();
    transient = null;
    const outcome = await guard(() => invoke("capture_snip"));
    if (outcome && ui.saveStateText) ui.saveStateText.textContent = "Screenshot saved";
    await refresh();
  }

  async function selectRegion() {
    stopRecording();
    transient = null;
    await guard(() => invoke("select_region"));
    await refresh();
  }

  async function selectWindow() {
    stopRecording();
    transient = null;
    await guard(() => invoke("select_window"));
    await refresh();
  }

  async function openFolder() {
    await guard(() => invoke("open_folder"));
  }

  async function chooseFolder() {
    const chosen = await guard(() =>
      invoke("choose_folder", { current: form.output_dir || null })
    );
    if (chosen) {
      form.output_dir = chosen;
      ui.outputDir.value = chosen;
      scheduleAutoSave(0);
    }
  }

  async function openWindowList() {
    ui.modal.hidden = false;
    ui.windowList.replaceChildren(emptyRow("Looking for windows…"));
    const windows = await guard(() => invoke("list_windows"));
    if (!windows) {
      ui.windowList.replaceChildren(emptyRow("The window list could not be read."));
      return;
    }
    if (windows.length === 0) {
      ui.windowList.replaceChildren(
        emptyRow("No capturable windows were found. Open the application first.")
      );
      return;
    }
    windows.sort(
      (a, b) => Number(Boolean(b.active)) - Number(Boolean(a.active))
    );
    ui.windowList.replaceChildren();
    for (const info of windows) {
      const target = info.target;
      const button = document.createElement("button");
      button.type = "button";
      button.className = "window-item";

      const title = document.createElement("span");
      title.className = "win-title";
      title.textContent =
        target.title || target.class || `window ${target.window_id}`;
      title.title = `${target.title || target.class} · ${target.class}`;

      const meta = document.createElement("span");
      meta.className = "win-meta";
      meta.textContent = `${info.geometry.width}×${info.geometry.height}`;

      button.append(title);
      if (info.active) {
        const active = document.createElement("span");
        active.className = "win-active";
        active.textContent = "FOCUSED";
        button.append(active);
      }
      if (info.minimized) {
        const minimized = document.createElement("span");
        minimized.className = "win-meta";
        minimized.textContent = "minimised";
        button.append(minimized);
      }
      button.append(meta);
      button.addEventListener("click", () => pinWindow(target));
      ui.windowList.append(button);
    }
  }

  async function pinWindow(target) {
    ui.modal.hidden = true;
    const next = await guard(() => invoke("pin_window", { target }));
    if (next) {
      snapshot = next;
      form = clone(next.settings);
      dirty = false;
      fillForm();
      render();
      if (ui.saveStateText) ui.saveStateText.textContent = "Window pinned";
    }
  }

  function emptyRow(text) {
    const node = document.createElement("div");
    node.className = "empty";
    node.textContent = text;
    return node;
  }

  // -------------------------------------------------------------------------
  // Shortcut recorder
  // -------------------------------------------------------------------------

  function prettyCode(code) {
    return code
      .replace(/^Key([A-Z])$/, "$1")
      .replace(/^Digit([0-9])$/, "$1")
      .replace(/^Arrow/, "");
  }

  function startRecording() {
    if (recording) return;
    recording = true;
    renderShortcut();
    document.addEventListener("keydown", onRecordKey, true);
  }

  function stopRecording() {
    if (!recording) return;
    recording = false;
    document.removeEventListener("keydown", onRecordKey, true);
    renderShortcut();
  }

  function onRecordKey(event) {
    event.preventDefault();
    event.stopPropagation();

    if (event.key === "Escape") {
      stopRecording();
      return;
    }
    if (["Control", "Alt", "Shift", "Meta"].includes(event.key)) {
      return;
    }
    if (event.key === "Backspace" || event.key === "Delete") {
      form.shortcut = "";
      stopRecording();
      scheduleAutoSave(0);
      return;
    }
    if (!KEY_PATTERN.test(event.code)) {
      ui.shortcutHelp.textContent =
        "That key cannot be used for a global shortcut. Try a letter, digit or function key.";
      return;
    }

    const modifiers = [];
    if (event.ctrlKey) modifiers.push("Ctrl");
    if (event.altKey) modifiers.push("Alt");
    if (event.shiftKey) modifiers.push("Shift");
    if (event.metaKey) modifiers.push("Super");

    if (modifiers.length === 0) {
      ui.shortcutHelp.textContent =
        "Hold at least one of Ctrl, Alt, Shift or Super together with the key.";
      return;
    }

    form.shortcut = [...modifiers, prettyCode(event.code)].join("+");
    stopRecording();
    scheduleAutoSave(0);
  }

  // -------------------------------------------------------------------------
  // Wiring
  // -------------------------------------------------------------------------

  document.querySelectorAll('input[name="mode"]').forEach((radio) => {
    radio.addEventListener("change", () => {
      if (!radio.checked) return;
      form.mode = radio.value;
      render();
      scheduleAutoSave(0);
    });
  });

  ui.outputDir.addEventListener("input", () => {
    form.output_dir = ui.outputDir.value;
    scheduleAutoSave(400);
  });

  ui.pattern.addEventListener("input", () => {
    form.filename_pattern = ui.pattern.value;
    renderPreview();
    scheduleAutoSave(400);
  });

  ui.quality.addEventListener("input", () => {
    form.jpeg_quality = Number(ui.quality.value);
    renderQuality();
    scheduleAutoSave(200);
  });

  ui.autostart.addEventListener("change", () => {
    form.launch_at_login = ui.autostart.checked;
    scheduleAutoSave(0);
  });

  ui.showToast.addEventListener("change", () => {
    form.show_toast = ui.showToast.checked;
    scheduleAutoSave(0);
  });

  ui.format.addEventListener("click", (event) => {
    const button = event.target.closest("button[data-format]");
    if (!button) return;
    form.format = button.dataset.format;
    renderFormat();
    renderPreview();
    scheduleAutoSave(0);
  });

  ui.browse.addEventListener("click", chooseFolder);
  ui.openFolder.addEventListener("click", openFolder);
  ui.openFolderInline.addEventListener("click", openFolder);
  ui.capture.addEventListener("click", captureNow);
  if (ui.snipNow) {
    ui.snipNow.addEventListener("click", captureSnip);
  }
  ui.selectRegion.addEventListener("click", selectRegion);
  ui.selectWindow.addEventListener("click", selectWindow);
  ui.chooseWindow.addEventListener("click", openWindowList);
  ui.recordShortcut.addEventListener("click", startRecording);
  ui.clearShortcut.addEventListener("click", () => {
    form.shortcut = "";
    renderShortcut();
    scheduleAutoSave(0);
  });
  ui.hide.addEventListener("click", () => guard(() => invoke("hide_window")));
  ui.quit.addEventListener("click", () => guard(() => invoke("quit_app")));

  ui.modal.addEventListener("click", (event) => {
    if (event.target.dataset && event.target.dataset.close) ui.modal.hidden = true;
  });
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && !ui.modal.hidden) ui.modal.hidden = true;
  });

  document.addEventListener("dragover", (event) => event.preventDefault());
  document.addEventListener("drop", (event) => event.preventDefault());

  async function init() {
    await listen("state", (event) => applySnapshot(event.payload));
    const first = await invoke("get_state");
    applySnapshot(first, { adopt: true });
  }

  init().catch(showError);
})();
