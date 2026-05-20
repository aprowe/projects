import { invoke, Channel } from "@tauri-apps/api/core";
import { LatencyTracker, fmtUs } from "./latency.js";

// Binary wire format from the Rust side (little-endian, 32-byte header):
//   [u32 width]
//   [u32 height]
//   [i64 pts_us]
//   [i64 capture_unix_us]   — Rust wall-clock when decoder produced the frame
//   [i64 send_unix_us]      — Rust wall-clock immediately before channel.send
//   [rgba bytes...]
const HEADER_LEN = 4 + 4 + 8 + 8 + 8;

const canvas = document.getElementById("stage");
const ctx = canvas.getContext("2d", { alpha: false, desynchronized: true });
const status = document.getElementById("status");
const hud = document.getElementById("hud");

let imageData = null;
let frameCount = 0;
let lastTick = performance.now();
let fps = 0;

const tracker = new LatencyTracker(256);

// Convert performance high-res time to UNIX microseconds so we can subtract
// directly against the Rust timestamps in the header. timeOrigin is ms since
// epoch; performance.now() is ms since timeOrigin. Resolution is sub-ms.
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
  // pts at offset 8
  const captureUs = Number(view.getBigInt64(16, true));
  const sendUs = Number(view.getBigInt64(24, true));

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

  frameCount++;
  const now = performance.now();
  if (now - lastTick >= 500) {
    fps = (frameCount * 1000) / (now - lastTick);
    frameCount = 0;
    lastTick = now;
    renderHud(width, height);
  }
};

function renderHud(width, height) {
  status.textContent = `${width}x${height} — ${fps.toFixed(1)} fps`;

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
    `<span class="label">samples</span>      <span class="val">${s.n}</span>`,
    "",
    line("decode→send", s.decodeToSend),
    line("send→recv",   s.sendToRecv),
    line("recv→paint",  s.recvToPaint),
    line("total",       s.total),
  ].join("\n");
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
