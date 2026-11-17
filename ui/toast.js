/* Confirmation toast. Rust owns the auto-hide timer, this only renders. */
(() => {
  "use strict";

  const tauri = window.__TAURI__;
  if (!tauri) return;

  const card = document.getElementById("card");
  const icon = document.getElementById("icon");
  const title = document.getElementById("title");
  const message = document.getElementById("message");
  const detail = document.getElementById("detail");

  const ICONS = { success: "✓", error: "!", info: "i" };

  tauri.event.listen("toast", (event) => {
    const payload = event.payload || {};
    const tone = ["success", "error", "info"].includes(payload.tone)
      ? payload.tone
      : "info";

    card.className = tone;
    icon.textContent = ICONS[tone];
    title.textContent = payload.title || "Continuous Screenshot";
    message.textContent = payload.message || "";
    detail.textContent = payload.detail || "";
    detail.hidden = !payload.detail;

    // Restart the entrance transition even when a toast replaces another one.
    card.classList.remove("show");
    void card.offsetWidth;
    card.classList.add("show");
  });
})();
