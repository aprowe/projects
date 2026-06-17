//! Capture a simulation run as a self-contained HTML replay file.
//!
//! `ReplayRenderer` wraps an `AsciiRenderer` configuration plus a
//! buffer of captured frames. After every tick it asks the inner
//! renderer for its frame-as-string, and reads the engine event log
//! for any narration sentences this tick. On `flush_html()` the
//! buffered frames are serialized to JSON and embedded into a
//! mobile-friendly HTML page with play/pause/scrub controls.

use std::path::PathBuf;

use bevy_ecs::prelude::World;
use fortress_engine::{narrate, AsciiRenderer, EventLog, Renderer, Tick};
use serde::Serialize;

#[derive(Serialize)]
struct ZSlice {
    z: i32,
    ascii: String,
}

#[derive(Serialize)]
struct Frame {
    tick: u64,
    /// One entry per Z-level captured. Single-floor scenarios
    /// produce a single slice; multi-story scenarios produce one
    /// slice per floor and the viewer exposes a Z scrubber.
    slices: Vec<ZSlice>,
    narration: Vec<String>,
}

pub struct ReplayRenderer {
    inner: AsciiRenderer,
    /// Z-levels to capture each frame. Defaults to `[inner.z]` so
    /// single-floor scenarios behave exactly as before.
    z_slices: Vec<i32>,
    frames: Vec<Frame>,
    output_path: PathBuf,
    title: String,
}

impl ReplayRenderer {
    pub fn new(inner: AsciiRenderer, output_path: PathBuf, title: impl Into<String>) -> Self {
        let z = inner.z;
        Self {
            inner,
            z_slices: vec![z],
            frames: Vec::new(),
            output_path,
            title: title.into(),
        }
    }

    /// Capture multiple Z-levels per frame. Each call replaces the
    /// existing list. Z-levels render top-down in the viewer (the
    /// last entry shows first).
    pub fn with_z_slices(mut self, slices: impl IntoIterator<Item = i32>) -> Self {
        self.z_slices = slices.into_iter().collect();
        if self.z_slices.is_empty() {
            self.z_slices.push(self.inner.z);
        }
        self
    }

    /// Write the buffered frames as a self-contained HTML file. Call
    /// after the run finishes.
    pub fn flush_html(&self) -> std::io::Result<()> {
        let frames_json = serde_json::to_string(&self.frames).map_err(std::io::Error::other)?;
        let html = HTML_TEMPLATE
            .replace("__TITLE__", &html_escape(&self.title))
            .replace("__FRAMES_JSON__", &frames_json);
        std::fs::write(&self.output_path, html)?;
        Ok(())
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }
}

impl Renderer for ReplayRenderer {
    fn frame(&mut self, world: &mut World, tick: Tick) {
        let mut slices: Vec<ZSlice> = Vec::with_capacity(self.z_slices.len());
        let original_z = self.inner.z;
        let z_levels = self.z_slices.clone();
        let mut any_rendered = false;
        for z in z_levels {
            self.inner.z = z;
            if let Some(ascii) = self.inner.frame_to_html(world, tick) {
                slices.push(ZSlice { z, ascii });
                any_rendered = true;
            }
        }
        self.inner.z = original_z;
        if !any_rendered {
            return;
        }
        let narration: Vec<String> = {
            let log = world.resource::<EventLog>();
            let events = log
                .events_at(tick)
                .cloned()
                .collect::<Vec<fortress_engine::Event>>();
            events
                .iter()
                .map(|e| narrate(e, world))
                .filter(|s| !s.is_empty())
                .collect()
        };
        self.frames.push(Frame {
            tick,
            slices,
            narration,
        });
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const HTML_TEMPLATE: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>__TITLE__</title>
<style>
:root {
  --bg: #0d1117;
  --fg: #c9d1d9;
  --muted: #8b949e;
  --accent: #58a6ff;
  --panel: #161b22;
  --border: #30363d;
}
* { box-sizing: border-box; }
html, body { margin: 0; padding: 0; height: 100%; }
body {
  background: var(--bg);
  color: var(--fg);
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif;
  font-size: 16px;
  line-height: 1.4;
  display: flex;
  flex-direction: column;
  min-height: 100vh;
}
header {
  padding: 12px 16px;
  border-bottom: 1px solid var(--border);
  background: var(--panel);
}
h1 {
  margin: 0;
  font-size: 18px;
  font-weight: 600;
}
header .meta { color: var(--muted); font-size: 13px; margin-top: 2px; }
main {
  flex: 1;
  display: flex;
  flex-direction: column;
  padding: 12px;
  gap: 12px;
}
.tickline {
  color: var(--muted);
  font-size: 14px;
  font-variant-numeric: tabular-nums;
}
.ascii {
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 12px;
  margin: 0;
  font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, monospace;
  font-size: 14px;
  line-height: 1.0;
  overflow-x: auto;
  color: var(--fg);
}
.ascii .meta { color: var(--muted); margin-bottom: 8px; }
.ascii .map { font-size: 0; line-height: 0; }
.ascii .row { font-size: 14px; line-height: 1.0; white-space: nowrap; }
.ascii .row span { display: inline-block; width: 0.7em; }
.narration {
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 12px;
  font-size: 15px;
  min-height: 60px;
}
.narration ul {
  margin: 0;
  padding-left: 20px;
}
.narration li { margin: 0 0 4px 0; }
.narration .empty { color: var(--muted); font-style: italic; }
.controls {
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 10px;
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  align-items: center;
  position: sticky;
  bottom: 8px;
}
.controls button {
  background: var(--bg);
  color: var(--fg);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 8px 12px;
  font-size: 14px;
  font-family: inherit;
  cursor: pointer;
  min-width: 44px;
}
.controls button:hover { border-color: var(--accent); }
.controls button.primary { background: var(--accent); color: #0d1117; border-color: var(--accent); }
.controls .speed { display: flex; align-items: center; gap: 4px; }
.controls input[type=range] {
  flex: 1;
  min-width: 140px;
  accent-color: var(--accent);
}
.controls select {
  background: var(--bg);
  color: var(--fg);
  border: 1px solid var(--border);
  border-radius: 4px;
  padding: 4px 6px;
  font-size: 14px;
  font-family: inherit;
}
@media (max-width: 600px) {
  pre.ascii { font-size: 12px; }
  .controls { font-size: 13px; padding: 8px; gap: 6px; }
  .controls button { padding: 6px 10px; }
}
</style>
</head>
<body>
<header>
  <h1>__TITLE__</h1>
  <div class="meta" id="meta"></div>
</header>
<main>
  <div class="tickline" id="tickline">tick 0</div>
  <div class="ascii" id="ascii"></div>
  <div class="narration" id="narration"></div>
  <div class="controls">
    <button id="prev" aria-label="previous tick">⏮</button>
    <button id="play" class="primary" aria-label="play/pause">▶</button>
    <button id="next" aria-label="next tick">⏭</button>
    <input type="range" id="scrub" min="0" max="0" value="0">
    <span class="speed">
      <label for="speed">speed</label>
      <select id="speed">
        <option value="1500">0.7×</option>
        <option value="1000" selected>1×</option>
        <option value="500">2×</option>
        <option value="200">5×</option>
        <option value="80">12×</option>
      </select>
    </span>
    <span class="zlevel" id="zwrap" style="display:none">
      <label for="zsel">floor</label>
      <select id="zsel"></select>
    </span>
  </div>
</main>
<script>
const FRAMES = __FRAMES_JSON__;
const $ = (id) => document.getElementById(id);
const meta = $("meta");
const tickline = $("tickline");
const asciiEl = $("ascii");
const narrEl = $("narration");
const scrub = $("scrub");
const playBtn = $("play");
const prevBtn = $("prev");
const nextBtn = $("next");
const speedSel = $("speed");

let idx = 0;
let timer = null;
let playing = false;
let zIdx = 0;

const zsel = $("zsel");
const zwrap = $("zwrap");

function render() {
  const f = FRAMES[idx];
  if (!f) return;
  // Pick a slice. Multi-floor frames carry `slices`; legacy frames
  // would carry a single `ascii` string.
  const slices = f.slices || (f.ascii ? [{z: 0, ascii: f.ascii}] : []);
  if (slices.length > 1 && zsel.options.length !== slices.length) {
    zsel.replaceChildren();
    slices.forEach((s, i) => {
      const opt = document.createElement("option");
      opt.value = String(i);
      opt.textContent = `z=${s.z}`;
      zsel.appendChild(opt);
    });
    zwrap.style.display = "inline-flex";
  } else if (slices.length <= 1) {
    zwrap.style.display = "none";
  }
  if (zIdx >= slices.length) zIdx = 0;
  zsel.value = String(zIdx);
  tickline.textContent = `tick ${f.tick}  (frame ${idx + 1}/${FRAMES.length})`;
  asciiEl.innerHTML = slices[zIdx] ? slices[zIdx].ascii : "";
  if (f.narration && f.narration.length > 0) {
    const ul = document.createElement("ul");
    f.narration.forEach(s => {
      const li = document.createElement("li");
      li.textContent = s;
      ul.appendChild(li);
    });
    narrEl.replaceChildren(ul);
  } else {
    narrEl.replaceChildren(Object.assign(document.createElement("span"), { className: "empty", textContent: "(quiet — no events this tick)" }));
  }
  scrub.value = idx;
}

function step(by) {
  idx = Math.max(0, Math.min(FRAMES.length - 1, idx + by));
  render();
}

function tick() {
  if (idx >= FRAMES.length - 1) {
    pause();
    return;
  }
  step(1);
}

function play() {
  if (playing) return;
  if (idx >= FRAMES.length - 1) idx = 0;
  playing = true;
  playBtn.textContent = "⏸";
  const interval = parseInt(speedSel.value, 10);
  timer = setInterval(tick, interval);
}

function pause() {
  playing = false;
  playBtn.textContent = "▶";
  if (timer) clearInterval(timer);
  timer = null;
}

playBtn.addEventListener("click", () => playing ? pause() : play());
prevBtn.addEventListener("click", () => { pause(); step(-1); });
nextBtn.addEventListener("click", () => { pause(); step(1); });
scrub.addEventListener("input", (e) => {
  pause();
  idx = parseInt(e.target.value, 10);
  render();
});
speedSel.addEventListener("change", () => {
  if (playing) { pause(); play(); }
});
zsel.addEventListener("change", (e) => {
  zIdx = parseInt(e.target.value, 10);
  render();
});
document.addEventListener("keydown", (e) => {
  if (e.key === " " || e.key === "Enter") { e.preventDefault(); playing ? pause() : play(); }
  else if (e.key === "ArrowRight") { pause(); step(1); }
  else if (e.key === "ArrowLeft") { pause(); step(-1); }
});

// Initialize
scrub.max = FRAMES.length - 1;
meta.textContent = `${FRAMES.length} frames captured`;
render();
</script>
</body>
</html>"##;
