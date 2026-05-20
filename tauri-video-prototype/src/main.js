import { invoke, Channel } from "@tauri-apps/api/core";

// Binary wire format from the Rust side (little-endian):
//   [u32 width][u32 height][i64 pts_us][rgba bytes...]
const HEADER_LEN = 4 + 4 + 8;

const canvas = document.getElementById("stage");
const ctx = canvas.getContext("2d", { alpha: false, desynchronized: true });
const status = document.getElementById("status");

let imageData = null;
let frameCount = 0;
let lastTick = performance.now();

const channel = new Channel();
channel.onmessage = (msg) => {
  // Tauri ships `InvokeResponseBody::Raw` as an ArrayBuffer here.
  const buf = msg instanceof ArrayBuffer ? msg : msg.buffer ?? null;
  if (!buf) return;

  const view = new DataView(buf);
  const width = view.getUint32(0, true);
  const height = view.getUint32(4, true);
  // pts_us at offset 8 (i64) — not used for draw, just for diagnostics.

  if (!imageData || imageData.width !== width || imageData.height !== height) {
    canvas.width = width;
    canvas.height = height;
    imageData = ctx.createImageData(width, height);
  }

  const pixels = new Uint8ClampedArray(buf, HEADER_LEN, width * height * 4);
  imageData.data.set(pixels);
  ctx.putImageData(imageData, 0, 0);

  frameCount++;
  const now = performance.now();
  if (now - lastTick >= 1000) {
    const fps = (frameCount * 1000) / (now - lastTick);
    status.textContent = `${width}x${height} — ${fps.toFixed(1)} fps`;
    frameCount = 0;
    lastTick = now;
  }
};

invoke("start_stream", { channel })
  .then(() => {
    status.textContent = "stream started";
  })
  .catch((err) => {
    status.textContent = `error: ${err}`;
    console.error(err);
  });
