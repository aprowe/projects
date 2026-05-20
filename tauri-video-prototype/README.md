# tauri-video-prototype

Low-latency video playback prototype:

- **Backend (Rust / Tauri)**: decodes a video with `ffmpeg-next`, converts
  frames to RGBA, and streams them over a Tauri v2 `Channel<InvokeResponseBody>`
  as raw binary IPC payloads (no JSON / base64 round-trip).
- **Frontend (HTML + JS canvas)**: receives each frame as an `ArrayBuffer`,
  wraps it in an `ImageData`, and `putImageData`s it straight onto a
  `<canvas>`. No `<video>` element, no MSE — one decode hop and one draw hop.

## Latency choices

- `ffmpeg-next` decodes directly to RGBA via `sws_scale`, sized to the
  source resolution. The frontend canvas is resized to match, so there is
  no per-frame software resize on the JS side.
- Frames are shipped over Tauri's `Channel`, which serializes binary as a
  raw byte payload (not JSON-encoded). This avoids base64 and avoids the
  `emit`-style broadcast machinery.
- Decoding runs in a dedicated thread so the Tauri main event loop never
  blocks on demux/decode.
- The frontend uses `requestAnimationFrame`-free draws: the channel
  callback paints immediately, so the only buffering is the OS compositor.

## Playback rate (sine-wave mode)

The decoder paces frames in wall-clock time. Speed is computed as

```
speed(t) = max(MIN_SPEED, base_speed + amplitude * sin(2π t / period_s))
```

where `t` is seconds since the current mode was selected. Presets live in
the top-right control panel:

| Mode          | base | amplitude | period |
| ------------- | ---- | --------- | ------ |
| realtime      | 1.0  | 0.0       | —      |
| sine · gentle | 1.0  | 0.5       | 6 s    |
| sine · wild   | 1.0  | 0.9       | 3 s    |
| sine · slow   | 1.0  | 0.5       | 12 s   |

Switching mode resets the wave's phase to 0 so the speed-up always starts
from 1.0×. The currently-applied speed is sent in every frame header and
shown on the HUD plus a live speed bar (0× left, 1× center, 2× right).

The custom Tauri command is:

```js
import { invoke } from "@tauri-apps/api/core";
await invoke("set_playback", {
  config: { base_speed: 1.0, amplitude: 0.7, period_s: 4.0 },
});
```

The pacer uses a "target wall time" scheme — it sleeps until the planned
send instant rather than sleeping a per-frame delta — so it doesn't
accumulate scheduler jitter. If it falls more than 250 ms behind (e.g.
after a big speed-up) it resyncs instead of sprinting through a backlog.

## Latency detector

Every frame carries two Rust wall-clock timestamps in its header:

| Field              | Captured when                              |
| ------------------ | ------------------------------------------ |
| `capture_unix_us`  | Decoder produced the frame (post-`receive_frame`) |
| `send_unix_us`     | Immediately before `channel.send`          |

The frontend records two more (`recv_unix_us` at the `onmessage` entry,
`paint_unix_us` right after `putImageData`) and computes:

- `decode→send` — Rust-side: scale + RGBA copy + IPC serialize
- `send→recv`   — cross-boundary: Tauri IPC + WebView main thread wake
- `recv→paint`  — JS-side: header parse + ImageData blit
- `total`       — `paint_us - capture_us`

The HUD shows mean / p50 / p99 / max over a 256-sample ring. The bar
turns yellow at >20 ms p99 and red at >50 ms p99.

**Clock alignment.** Both sides use system wall clock (`SystemTime` in
Rust, `performance.timeOrigin + performance.now()` in JS) so the
timestamps are directly subtractable without a sync handshake. NTP
adjustments mid-stream would produce negative deltas; those samples are
dropped rather than smoothed.

**What `recv→paint` does NOT measure.** `putImageData` returns when the
draw is queued, not when the pixels hit the panel. For
display-to-photons latency, point a high-speed camera at the canvas; the
top-left pixel can be made a frame counter (TODO) for visual sync.

## Running

```bash
# 1. Native deps (Ubuntu/Debian):
sudo apt-get install -y libavcodec-dev libavformat-dev libavutil-dev \
    libswscale-dev libavfilter-dev libavdevice-dev pkg-config clang

# 2. Frontend deps + Tauri CLI:
npm install

# 3. Run in dev mode, pointing at any file ffmpeg can open:
VIDEO_PATH=/path/to/sample.mp4 npm run tauri dev
```

The path can also be an RTSP/HTTP URL — anything `avformat_open_input`
accepts.
