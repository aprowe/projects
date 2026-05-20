import { invoke, Channel } from "@tauri-apps/api/core";
import { LatencyTracker, fmtUs } from "./latency.js";

// Binary wire format from the Rust side (little-endian, 40-byte header):
//   [u32 width]
//   [u32 height]
//   [i64 pts_us]
//   [i64 capture_unix_us]
//   [i64 send_unix_us]
//   [f64 playback_speed]
//   [rgba bytes...]
const HEADER_LEN = 4 + 4 + 8 + 8 + 8 + 8;

const PLAYBACK_PRESETS = {
  "realtime":    { base_speed: 1.0, amplitude: 0.0, period_s: 6.0 },
  "sine-gentle": { base_speed: 1.0, amplitude: 0.5, period_s: 6.0 },
  "sine-wild":   { base_speed: 1.0, amplitude: 0.9, period_s: 3.0 },
  "sine-slow":   { base_speed: 1.0, amplitude: 0.5, period_s: 12.0 },
};
// Speed bar mapping: 0× → left edge, 1× → middle, 2× → right edge.
const SPEED_BAR_MAX = 2.0;

const canvas = document.getElementById("stage");
const ctx = canvas.getContext("2d", { alpha: false, desynchronized: true });
const status = document.getElementById("status");
const hud = document.getElementById("hud");
const speedText = document.getElementById("speed-text");
const speedFill = document.querySelector("#speed-bar .fill");

let imageData = null;
let frameCount = 0;
let lastTick = performance.now();
let fps = 0;
let lastSpeed = 1.0;

const tracker = new LatencyTracker(256);

function nowUnixUs() {
  return Math.round((performance.timeOrigin + performance.now()) * 1000);
}

const channel = new Channel();
channel.onmessage = (msg) => {
  const recvUs = nowUnixUs();

  const buf = msg instanceof ArrayBuffer ? msg : msg?.buffer ?? null;
  if (!buf) return;

  const view = new DataView(buf);
  const width = view.getUint32(0, true);
  const height = view.getUint32(4, true);
  const captureUs = Number(view.getBigInt64(16, true));
  const sendUs = Number(view.getBigInt64(24, true));
  const speed = view.getFloat64(32, true);

  if (!imageData || imageData.width !== width || imageData.height !== height) {
    canvas.width = width;
    canvas.height = height;
    imageData = ctx.createImageData(width, height);
  }

  const pixels = new Uint8ClampedArray(buf, HEADER_LEN, width * height * 4);
  imageData.data.set(pixels);
  ctx.putImageData(imageData, 0, 0);

  const paintUs = nowUnixUs();

  tracker.record({
    decodeToSend: sendUs - captureUs,
    sendToRecv: recvUs - sendUs,
    recvToPaint: paintUs - recvUs,
    total: paintUs - captureUs,
  });

  lastSpeed = speed;
  updateSpeedBar(speed);

  frameCount++;
  const now = performance.now();
  if (now - lastTick >= 500) {
    fps = (frameCount * 1000) / (now - lastTick);
    frameCount = 0;
    lastTick = now;
    renderHud(width, height);
  }
};

function updateSpeedBar(speed) {
  const pct = Math.min(100, Math.max(0, (speed / SPEED_BAR_MAX) * 100));
  speedFill.style.width = `${pct}%`;
  speedText.textContent = `speed ${speed.toFixed(2)}×`;
}

function renderHud(width, height) {
  status.textContent = `${width}x${height} — ${fps.toFixed(1)} fps · ${lastSpeed.toFixed(2)}×`;

  const s = tracker.snapshot();
  if (!s) {
    hud.textContent = "warming up…";
    return;
  }

  const line = (label, stat) => {
    const cls = stat.p99 > 50_000 ? "bad" : stat.p99 > 20_000 ? "warn" : "val";
    return (
      `<span class="label">${label.padEnd(13)}</span>` +
      `<span class="${cls}">` +
      `mean ${fmtUs(stat.mean).padStart(7)}  ` +
      `p50 ${fmtUs(stat.p50).padStart(7)}  ` +
      `p99 ${fmtUs(stat.p99).padStart(7)}  ` +
      `max ${fmtUs(stat.max).padStart(7)}` +
      `</span>`
    );
  };

  hud.innerHTML = [
    `<span class="label">resolution</span>   <span class="val">${width}x${height}</span>`,
    `<span class="label">fps</span>          <span class="val">${fps.toFixed(1)}</span>`,
    `<span class="label">speed</span>        <span class="val">${lastSpeed.toFixed(2)}×</span>`,
    `<span class="label">samples</span>      <span class="val">${s.n}</span>`,
    "",
    line("decode→send", s.decodeToSend),
    line("send→recv",   s.sendToRecv),
    line("recv→paint",  s.recvToPaint),
    line("total",       s.total),
  ].join("\n");
}

for (const btn of document.querySelectorAll("#controls button[data-mode]")) {
  btn.addEventListener("click", async () => {
    const mode = btn.dataset.mode;
    const config = PLAYBACK_PRESETS[mode];
    if (!config) return;
    try {
      await invoke("set_playback", { config });
      for (const other of document.querySelectorAll("#controls button[data-mode]")) {
        other.classList.toggle("active", other === btn);
      }
    } catch (err) {
      console.error("set_playback failed", err);
    }
  });
}

invoke("start_stream", { channel })
  .then(() => {
    status.textContent = "stream started";
  })
  .catch((err) => {
    status.textContent = `error: ${err}`;
    hud.textContent = `error: ${err}`;
    console.error(err);
  });
