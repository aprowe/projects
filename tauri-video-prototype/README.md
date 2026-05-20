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
