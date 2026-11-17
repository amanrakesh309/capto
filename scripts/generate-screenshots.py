#!/usr/bin/env python3
"""Generate crisp, high-DPI screenshots of Capto for documentation."""

import os
import subprocess
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
SCREENSHOTS_DIR = REPO_ROOT / "docs" / "screenshots"
SCREENSHOTS_DIR.mkdir(parents=True, exist_ok=True)

CSS_FILE = REPO_ROOT / "ui" / "styles.css"
TRAY_ICON_FILE = REPO_ROOT / "src-tauri" / "icons" / "tray.png"

def render_html_to_png(html_content: str, output_path: Path, width: int, height: int, dpr: float = 2.0):
    """Render an HTML string to PNG via headless Chrome."""
    with tempfile.NamedTemporaryFile("w", suffix=".html", delete=False) as f:
        f.write(html_content)
        temp_path = f.name

    cmd = [
        "google-chrome",
        "--headless=new",
        f"--window-size={width},{height}",
        f"--force-device-scale-factor={dpr}",
        "--hide-scrollbars",
        f"--screenshot={str(output_path)}",
        f"file://{temp_path}",
    ]
    try:
        subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        print(f"✓ Generated {output_path.name} ({width*int(dpr)}x{height*int(dpr)})")
    finally:
        if os.path.exists(temp_path):
            os.remove(temp_path)


def generate_main_window_screenshot():
    css = CSS_FILE.read_text()
    
    html = f"""<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>Capto</title>
  <style>
    {css}
    
    /* Window frame container for screenshot */
    body {{
      background: #1e1f29 !important;
      padding: 30px;
      display: flex;
      justify-content: center;
      align-items: center;
      min-height: 100vh;
      box-sizing: border-box;
    }}
    .window-frame {{
      width: 760px;
      background: var(--bg);
      border-radius: 16px;
      box-shadow: 0 25px 50px -12px rgba(0, 0, 0, 0.7), 0 0 0 1px rgba(255, 255, 255, 0.08);
      overflow: hidden;
      display: flex;
      flex-direction: column;
    }}
    .window-titlebar {{
      height: 40px;
      background: #21222c;
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 0 16px;
      border-bottom: 1px solid var(--line);
      user-select: none;
    }}
    .window-titlebar-title {{
      font-size: 12px;
      font-weight: 600;
      color: var(--muted);
      letter-spacing: 0.3px;
    }}
    .window-controls {{
      display: flex;
      gap: 8px;
    }}
    .win-btn {{
      width: 12px;
      height: 12px;
      border-radius: 50%;
      display: inline-block;
    }}
    .win-btn.close {{ background: #ff5555; }}
    .win-btn.min {{ background: #f1fa8c; }}
    .win-btn.max {{ background: #50fa7b; }}
    .window-frame > main {{
      padding: 24px;
      gap: 20px;
      overflow: visible;
    }}
    .window-frame > header {{
      padding: 16px 24px 0 24px;
    }}
    .window-frame > footer {{
      padding: 14px 24px;
    }}
  </style>
</head>
<body>
  <div class="window-frame">
    <div class="window-titlebar">
      <div class="window-controls">
        <span class="win-btn close"></span>
        <span class="win-btn min"></span>
        <span class="win-btn max"></span>
      </div>
      <div class="window-titlebar-title">Capto — Continuous &amp; Snip Screenshot Tool</div>
      <div style="width: 52px;"></div>
    </div>

    <header class="topbar">
      <div class="brand">
        <div class="brand-mark" aria-hidden="true">
          <span></span><span></span><span></span><span></span>
          <i></i>
        </div>
        <div>
          <h1>Capto</h1>
          <p id="mode-subtitle">Continuous &amp; Snip Screenshot Tool</p>
        </div>
      </div>
      <div id="status-pill" class="pill ready">Ready</div>
    </header>

    <main id="content">
      <!-- Main Capture & Mode Card -->
      <section class="card target-card">
        <div class="card-head">
          <div>
            <h2>Capture</h2>
            <p id="target-summary" class="target-title">📌 Pinned window: Visual Studio Code (Capto — app.tsx)</p>
          </div>
          <span id="mode-badge" class="badge pinned">Pinned Window</span>
        </div>

        <!-- Primary Trigger Actions -->
        <div class="actions primary-actions">
          <button id="capture-now" class="btn primary">
            <span class="kbd-dot"></span> Capture now
          </button>
          <button id="snip-now" class="btn accent">
            <span class="snip-icon">✂</span> Snip area
          </button>
          <span id="contextual-actions" class="contextual-actions">
            <button id="select-window" class="btn tiny">Pin window…</button>
            <button id="choose-window" class="btn tiny ghost">Pick from list…</button>
          </span>
        </div>

        <!-- Mode Grid Selector -->
        <div class="section-label">Capture Mode</div>
        <div id="modes" class="mode-grid">
          <label class="mode-chip" data-mode="select_area">
            <input type="radio" name="mode" value="select_area" />
            <span class="chip-content">
              <strong>✂ Snip Area</strong>
              <span class="chip-desc">Drag to snip on capture</span>
            </span>
          </label>
          <label class="mode-chip" data-mode="region">
            <input type="radio" name="mode" value="region" />
            <span class="chip-content">
              <strong>⛶ Fixed Region</strong>
              <span class="chip-desc">Remembered area</span>
            </span>
          </label>
          <label class="mode-chip" data-mode="full_screen">
            <input type="radio" name="mode" value="full_screen" />
            <span class="chip-content">
              <strong>🖥 Full Screen</strong>
              <span class="chip-desc">Entire desktop</span>
            </span>
          </label>
          <label class="mode-chip active" data-mode="pinned_window">
            <input type="radio" name="mode" value="pinned_window" checked />
            <span class="chip-content">
              <strong>📌 Pinned Window</strong>
              <span class="chip-desc">Specific app</span>
            </span>
          </label>
          <label class="mode-chip" data-mode="foreground_window">
            <input type="radio" name="mode" value="foreground_window" />
            <span class="chip-content">
              <strong>🗔 Active Window</strong>
              <span class="chip-desc">Focused app</span>
            </span>
          </label>
        </div>

        <!-- Last Capture Result -->
        <div id="last-capture" class="last-capture">
          <div class="last-icon" aria-hidden="true">✓</div>
          <div class="last-body">
            <div class="last-title" id="last-filename">Screenshot_2026-11-17_06-30-15.png</div>
            <div class="muted small" id="last-meta">1920×1080 • 248 KB • Saved to ~/Pictures/Screenshots</div>
          </div>
          <button id="open-folder-inline" class="btn tiny ghost">Open folder</button>
        </div>
      </section>

      <!-- Output Settings -->
      <section class="card">
        <div class="card-head">
          <h2>Storage &amp; Format</h2>
        </div>

        <div class="field">
          <span class="field-label">Save Folder</span>
          <div class="row">
            <input id="output-dir" type="text" value="/home/aman/Pictures/Screenshots" readonly />
            <button id="browse" class="btn">Browse…</button>
            <button id="open-folder" class="btn ghost">Open</button>
          </div>
        </div>

        <div class="grid-2">
          <div class="field">
            <span class="field-label">Format</span>
            <div class="segmented" id="format">
              <button type="button" class="active" data-format="png">PNG <em>lossless</em></button>
              <button type="button" data-format="jpeg">JPEG <em>smaller</em></button>
            </div>
          </div>

          <div class="field" id="quality-field">
            <span class="field-label">
              JPEG Quality <output id="quality-value">90</output>%
            </span>
            <input id="quality" type="range" min="1" max="100" step="1" value="90" />
          </div>
        </div>

        <div class="field">
          <span class="field-label">Filename Pattern</span>
          <input id="pattern" type="text" value="Screenshot_{{date}}_{{time}}" />
          <span class="muted small">
            Preview: <span id="pattern-preview" class="mono">Screenshot_2026-11-17_06-30-15.png</span>
          </span>
        </div>
      </section>

      <!-- Shortcut & Preferences -->
      <section class="card">
        <div class="card-head">
          <h2>Shortcut &amp; Preferences</h2>
        </div>

        <div class="field">
          <span class="field-label">Global Shortcut</span>
          <div class="row shortcut-row">
            <div id="shortcut-display" class="shortcut mono">Ctrl+Alt+S</div>
            <button id="record-shortcut" class="btn">Change…</button>
            <button id="clear-shortcut" class="btn ghost">Disable</button>
          </div>
          <span id="shortcut-help" class="muted small">
            Press shortcut anytime across any application. Tray menu also stays available.
          </span>
        </div>

        <div class="switches-group">
          <label class="switch-row">
            <span>Start at login</span>
            <input id="autostart" type="checkbox" class="switch" checked />
          </label>
          <label class="switch-row">
            <span>Show capture notification</span>
            <input id="show-toast" type="checkbox" class="switch" checked />
          </label>
        </div>
      </section>

      <!-- Collapsible System Info -->
      <details class="card system-details">
        <summary>System &amp; Backend Info</summary>
        <dl class="facts">
          <div><dt>Backend</dt><dd id="fact-backend">X11 (Xlib/XShm)</dd></div>
          <div><dt>Display server</dt><dd id="fact-display">:0</dd></div>
          <div><dt>Desktop size</dt><dd id="fact-screen">2560×1440</dd></div>
          <div><dt>Settings file</dt><dd id="fact-config" class="mono">~/.config/io.github.capto/settings.json</dd></div>
        </dl>
      </details>
    </main>

    <footer class="footer">
      <div class="footer-left">
        <span id="save-state" class="autosave-pill small">
          <span class="autosave-dot"></span> <span id="save-state-text">Saved</span>
        </span>
      </div>
      <div class="footer-right">
        <span id="version" class="muted small">v0.1.0</span>
        <button id="hide" class="btn ghost">Hide window</button>
        <button id="quit" class="btn danger">Quit</button>
      </div>
    </footer>
  </div>
</body>
</html>"""
    output = SCREENSHOTS_DIR / "capto-main-window.png"
    render_html_to_png(html, output, width=840, height=1150)


def generate_tray_screenshot():
    css = CSS_FILE.read_text()
    
    html = f"""<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>Capto Tray</title>
  <style>
    {css}
    body {{
      background: #181920;
      padding: 0;
      margin: 0;
      display: flex;
      flex-direction: column;
      align-items: center;
      font-family: "Inter", "Segoe UI", system-ui, sans-serif;
    }}
    .desktop-panel {{
      width: 100%;
      height: 44px;
      background: #1e1f29;
      border-bottom: 1px solid #44475a;
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 0 20px;
      box-sizing: border-box;
      color: #f8f8f2;
      font-size: 13px;
    }}
    .panel-left {{
      display: flex;
      align-items: center;
      gap: 16px;
      font-weight: 500;
    }}
    .panel-right {{
      display: flex;
      align-items: center;
      gap: 14px;
    }}
    .tray-item {{
      display: flex;
      align-items: center;
      justify-content: center;
      height: 32px;
      padding: 0 8px;
      border-radius: 6px;
    }}
    .tray-item.active {{
      background: rgba(189, 147, 249, 0.25);
      border: 1px solid rgba(189, 147, 249, 0.4);
    }}
    .tray-camera-icon {{
      width: 22px;
      height: 22px;
      display: inline-block;
    }}
    
    /* Popover menu mockup */
    .menu-container {{
      width: 100%;
      max-width: 600px;
      display: flex;
      justify-content: flex-end;
      padding: 10px 40px 0 0;
      box-sizing: border-box;
    }}
    .tray-menu {{
      width: 280px;
      background: #282a36;
      border: 1px solid #6272a4;
      border-radius: 12px;
      padding: 8px 0;
      box-shadow: 0 16px 36px rgba(0, 0, 0, 0.65);
      color: #f8f8f2;
      font-size: 13.5px;
    }}
    .menu-header {{
      padding: 8px 16px 4px 16px;
      font-size: 11px;
      text-transform: uppercase;
      letter-spacing: 0.8px;
      color: #bd93f9;
      font-weight: 700;
    }}
    .menu-item {{
      padding: 8px 16px;
      display: flex;
      align-items: center;
      justify-content: space-between;
      cursor: pointer;
      transition: background 0.1s;
    }}
    .menu-item:hover, .menu-item.highlighted {{
      background: #44475a;
      color: #50fa7b;
    }}
    .menu-item .shortcut-label {{
      color: #6272a4;
      font-family: monospace;
      font-size: 11.5px;
      background: #21222c;
      padding: 2px 6px;
      border-radius: 4px;
      border: 1px solid #44475a;
    }}
    .menu-separator {{
      height: 1px;
      background: #44475a;
      margin: 6px 0;
    }}
    .menu-item.bold {{
      font-weight: 600;
    }}
  </style>
</head>
<body>
  <div class="desktop-panel">
    <div class="panel-left">
      <span>Activities</span>
      <span style="color: #6272a4;">|</span>
      <span>Tue Nov 17 06:30</span>
    </div>
    <div class="panel-right">
      <span style="color: #6272a4;">EN</span>
      <span style="color: #6272a4;">🔊</span>
      <span style="color: #6272a4;">📶</span>
      <div class="tray-item active">
        <img class="tray-camera-icon" src="file://{TRAY_ICON_FILE}" alt="Capto Tray Icon" />
      </div>
      <span>98%</span>
    </div>
  </div>

  <div class="menu-container">
    <div class="tray-menu">
      <div class="menu-header">Capto Actions</div>
      <div class="menu-item bold highlighted">
        <span>📸 Capture now</span>
        <span class="shortcut-label">Ctrl+Alt+S</span>
      </div>
      <div class="menu-item">
        <span>✂ Snip area now</span>
      </div>
      <div class="menu-item">
        <span>🖥 Capture full screen</span>
      </div>
      <div class="menu-separator"></div>
      <div class="menu-item">
        <span>⛶ Select region…</span>
      </div>
      <div class="menu-item">
        <span>📌 Pin a window…</span>
      </div>
      <div class="menu-item">
        <span>📁 Open screenshot folder</span>
      </div>
      <div class="menu-separator"></div>
      <div class="menu-item">
        <span>⏸ Pause capturing</span>
      </div>
      <div class="menu-item">
        <span>⚙ Settings…</span>
      </div>
      <div class="menu-separator"></div>
      <div class="menu-item" style="color: #ff5555;">
        <span>✕ Quit</span>
      </div>
    </div>
  </div>
</body>
</html>"""
    output = SCREENSHOTS_DIR / "capto-tray-menu.png"
    render_html_to_png(html, output, width=640, height=450)


def generate_snip_overlay_screenshot():
    html = f"""<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>Capto Snip Overlay</title>
  <style>
    body {{
      margin: 0;
      padding: 0;
      width: 100vw;
      height: 100vh;
      background: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='100' height='100' viewBox='0 0 100 100'%3E%3Crect fill='%23191a21' width='100' height='100'/%3E%3Cpath d='M0 50 H100 M50 0 V100' stroke='%23282a36' stroke-width='1'/%3E%3C/svg%3E");
      display: flex;
      justify-content: center;
      align-items: center;
      font-family: "Inter", "Segoe UI", system-ui, sans-serif;
      overflow: hidden;
      position: relative;
    }}

    /* Simulated code editor in background */
    .editor-bg {{
      position: absolute;
      top: 40px;
      left: 60px;
      right: 60px;
      bottom: 40px;
      background: #282a36;
      border-radius: 12px;
      box-shadow: 0 10px 30px rgba(0,0,0,0.5);
      border: 1px solid #44475a;
      padding: 24px;
      color: #f8f8f2;
      font-family: "Fira Code", monospace;
      font-size: 14px;
      line-height: 1.6;
      opacity: 0.65;
    }}

    /* Dimmed overlay */
    .overlay-scrim {{
      position: absolute;
      inset: 0;
      background: rgba(16, 17, 23, 0.55);
      backdrop-filter: blur(2px);
    }}

    /* Selected Snip Rectangle */
    .selection-box {{
      position: absolute;
      top: 130px;
      left: 140px;
      width: 580px;
      height: 340px;
      border: 2px solid #bd93f9;
      box-shadow: 0 0 0 9999px rgba(10, 11, 16, 0.45), 0 0 25px rgba(189, 147, 249, 0.4);
      border-radius: 4px;
      z-index: 10;
      pointer-events: none;
    }}

    .dimension-badge {{
      position: absolute;
      bottom: -32px;
      right: 0;
      background: #bd93f9;
      color: #282a36;
      font-weight: 700;
      font-size: 12px;
      padding: 4px 10px;
      border-radius: 6px;
      box-shadow: 0 4px 12px rgba(0,0,0,0.4);
    }}

    .selection-handle {{
      position: absolute;
      width: 10px;
      height: 10px;
      background: #ff79c6;
      border: 2px solid #ffffff;
      border-radius: 50%;
    }}
    .handle-tl {{ top: -6px; left: -6px; }}
    .handle-tr {{ top: -6px; right: -6px; }}
    .handle-bl {{ bottom: -6px; left: -6px; }}
    .handle-br {{ bottom: -6px; right: -6px; }}

    .crosshair-cursor {{
      position: absolute;
      bottom: -15px;
      right: -15px;
      width: 32px;
      height: 32px;
      display: flex;
      align-items: center;
      justify-content: center;
    }}
    .crosshair-h {{
      position: absolute;
      width: 24px;
      height: 2px;
      background: #50fa7b;
      box-shadow: 0 0 6px #50fa7b;
    }}
    .crosshair-v {{
      position: absolute;
      height: 24px;
      width: 2px;
      background: #50fa7b;
      box-shadow: 0 0 6px #50fa7b;
    }}

    .prompt-banner {{
      position: absolute;
      top: 40px;
      left: 50%;
      transform: translateX(-50%);
      background: #343746;
      border: 1px solid #bd93f9;
      color: #f8f8f2;
      padding: 10px 24px;
      border-radius: 20px;
      font-size: 14px;
      font-weight: 600;
      box-shadow: 0 8px 24px rgba(0,0,0,0.5);
      z-index: 20;
      display: flex;
      align-items: center;
      gap: 10px;
    }}
    .prompt-banner span.kbd {{
      background: #44475a;
      border: 1px solid #6272a4;
      padding: 2px 8px;
      border-radius: 4px;
      font-size: 11px;
      font-weight: 700;
    }}
  </style>
</head>
<body>
  <div class="editor-bg">
    <span style="color:#ff79c6">function</span> <span style="color:#50fa7b">captureRegion</span>(rect: <span style="color:#8be9fd">Rectangle</span>): <span style="color:#8be9fd">Promise&lt;CaptureResult&gt;</span> &#123;<br>
    &nbsp;&nbsp;<span style="color:#6272a4">// Instant snipping without remembering fixed bounds</span><br>
    &nbsp;&nbsp;<span style="color:#ff79c6">const</span> buffer = <span style="color:#ff79c6">await</span> backend.grabPixels(rect);<br>
    &nbsp;&nbsp;<span style="color:#ff79c6">return</span> storage.saveImage(buffer);<br>
    &#125;
  </div>

  <div class="overlay-scrim"></div>

  <div class="prompt-banner">
    <span>✂ Drag to snip area &bull; Releases to save immediately</span>
    <span class="kbd">ESC</span> to cancel
  </div>

  <div class="selection-box">
    <div class="selection-handle handle-tl"></div>
    <div class="selection-handle handle-tr"></div>
    <div class="selection-handle handle-bl"></div>
    <div class="selection-handle handle-br"></div>
    <div class="crosshair-cursor">
      <div class="crosshair-h"></div>
      <div class="crosshair-v"></div>
    </div>
    <div class="dimension-badge">960 × 540 px</div>
  </div>
</body>
</html>"""
    output = SCREENSHOTS_DIR / "capto-snip-overlay.png"
    render_html_to_png(html, output, width=860, height=560)


def generate_toast_screenshot():
    html = f"""<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>Capto Toast</title>
  <style>
    :root {{
      color-scheme: dark;
      --bg: #282a36;
      --card-bg: #343746;
      --line: #44475a;
      --line-strong: #6272a4;
      --text: #f8f8f2;
      --muted: #6272a4;
      --green: #50fa7b;
      --purple: #bd93f9;
      --pink: #ff79c6;
      --red: #ff5555;
      font-family: "Inter", "Segoe UI", "Ubuntu", system-ui, -apple-system, sans-serif;
    }}
    body {{
      margin: 0;
      padding: 16px;
      background: #181920;
      display: flex;
      justify-content: center;
      align-items: center;
      height: 100vh;
      box-sizing: border-box;
    }}
    #card {{
      width: 440px;
      display: flex;
      align-items: center;
      gap: 13px;
      padding: 14px 18px;
      border-radius: 15px;
      background: linear-gradient(180deg, rgba(52, 55, 70, 0.98), rgba(40, 42, 54, 0.98));
      border: 1px solid rgba(80, 250, 123, 0.4);
      box-shadow: 0 16px 38px rgba(0, 0, 0, 0.55);
      color: var(--text);
    }}
    #icon {{
      flex: 0 0 32px;
      width: 32px;
      height: 32px;
      border-radius: 50%;
      display: grid;
      place-items: center;
      font-size: 15px;
      font-weight: 700;
      color: var(--green);
      background: rgba(80, 250, 123, 0.16);
      border: 1px solid rgba(80, 250, 123, 0.38);
    }}
    .text {{
      min-width: 0;
      flex: 1 1 auto;
    }}
    .title {{
      font-size: 13.5px;
      font-weight: 650;
      letter-spacing: 0.1px;
    }}
    .message {{
      margin-top: 2px;
      font-size: 12px;
      color: #f8f8f2;
      font-family: monospace;
    }}
  </style>
</head>
<body>
  <div id="card" class="success">
    <div id="icon">✓</div>
    <div class="text">
      <div class="title">Screenshot saved</div>
      <div class="message">Screenshot_2026-11-17_06-30-15.png</div>
    </div>
  </div>
</body>
</html>"""
    output = SCREENSHOTS_DIR / "capto-toast.png"
    render_html_to_png(html, output, width=480, height=130)


if __name__ == "__main__":
    print("Generating screenshots...")
    generate_main_window_screenshot()
    generate_tray_screenshot()
    generate_snip_overlay_screenshot()
    generate_toast_screenshot()
    print("All screenshots successfully created in docs/screenshots/")
