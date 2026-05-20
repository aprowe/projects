//! ffmpeg-next based decoder that pushes RGBA frames through a Tauri
//! `Channel` as raw binary IPC messages.
//!
//! Wire format of each message (little-endian, no padding, 40-byte header):
//!   [u32 width]
//!   [u32 height]
//!   [i64 pts_us]            — presentation timestamp in the source stream
//!   [i64 capture_unix_us]   — wall-clock when decoder produced the frame
//!   [i64 send_unix_us]      — wall-clock immediately before channel.send
//!   [f64 playback_speed]    — speed multiplier applied to pace this frame
//!   [u8 * width * height * 4 rgba]

use std::f64::consts::TAU;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use ffmpeg::format::{input, Pixel};
use ffmpeg::media::Type;
use ffmpeg::software::scaling::{context::Context as Scaler, flag::Flags};
use ffmpeg::util::frame::video::Video;
use ffmpeg_next as ffmpeg;
use serde::Deserialize;
use tauri::ipc::{Channel, InvokeResponseBody, IpcResponse};

use crate::Playback;

/// Binary message pushed through the Tauri channel. We implement
/// `IpcResponse` so Tauri ships it as `InvokeResponseBody::Raw`, which the
/// JS side receives as an `ArrayBuffer` without JSON encoding.
pub struct FrameMessage(pub Vec<u8>);

impl IpcResponse for FrameMessage {
    fn body(self) -> tauri::Result<InvokeResponseBody> {
        Ok(InvokeResponseBody::Raw(self.0))
    }
}

const HEADER_LEN: usize = 4 + 4 + 8 + 8 + 8 + 8;
const MIN_SPEED: f64 = 0.05;

fn unix_us() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0)
}

/// Sine-wave modulated playback rate.
///
/// `speed(t) = clamp(base + amplitude * sin(2π t / period_s), MIN_SPEED, ∞)`
///
/// Defaults to constant 1.0x (amplitude 0) so playback is real-time until
/// the frontend opts into a sine mode.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(default)]
pub struct PlaybackConfig {
    pub base_speed: f64,
    pub amplitude: f64,
    pub period_s: f64,
}

impl Default for PlaybackConfig {
    fn default() -> Self {
        Self {
            base_speed: 1.0,
            amplitude: 0.0,
            period_s: 6.0,
        }
    }
}

impl PlaybackConfig {
    pub fn sanitized(self) -> Self {
        Self {
            base_speed: self.base_speed.max(MIN_SPEED),
            amplitude: self.amplitude.max(0.0),
            period_s: self.period_s.max(0.1),
        }
    }

    pub fn speed_at(&self, elapsed_s: f64) -> f64 {
        let raw = self.base_speed + self.amplitude * (TAU * elapsed_s / self.period_s).sin();
        raw.max(MIN_SPEED)
    }
}

pub struct DecoderHandle {
    cancel: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}

impl DecoderHandle {
    pub fn stop(mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(h) = self.join.take() {
            let _ = h.join();
        }
    }
}

impl Drop for DecoderHandle {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub fn spawn_decoder(
    path: String,
    channel: Channel<FrameMessage>,
    playback: Arc<Playback>,
) -> Result<DecoderHandle> {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_thread = cancel.clone();

    let join = thread::Builder::new()
        .name("video-decoder".into())
        .spawn(move || {
            if let Err(e) = decode_loop(&path, &channel, &cancel_thread, &playback) {
                eprintln!("decoder error: {e:#}");
            }
        })
        .context("spawning decoder thread")?;

    Ok(DecoderHandle {
        cancel,
        join: Some(join),
    })
}

fn decode_loop(
    path: &str,
    channel: &Channel<FrameMessage>,
    cancel: &AtomicBool,
    playback: &Arc<Playback>,
) -> Result<()> {
    let mut ictx = input(&path).with_context(|| format!("opening {path}"))?;

    let stream = ictx
        .streams()
        .best(Type::Video)
        .ok_or_else(|| anyhow!("no video stream"))?;
    let stream_index = stream.index();
    let time_base = stream.time_base();
    let avg_frame_rate = stream.avg_frame_rate();

    // Nominal microseconds per frame, used when a stream doesn't carry PTS
    // or jumps backward (e.g. wrap-around). Falls back to 30 fps.
    let nominal_frame_us: i64 = {
        let num = avg_frame_rate.numerator() as f64;
        let den = avg_frame_rate.denominator() as f64;
        if num > 0.0 && den > 0.0 {
            (1_000_000.0 * den / num) as i64
        } else {
            33_333
        }
    };

    let context_decoder = ffmpeg::codec::context::Context::from_parameters(stream.parameters())?;
    let mut decoder = context_decoder.decoder().video()?;

    let src_w = decoder.width();
    let src_h = decoder.height();
    let src_fmt = decoder.format();

    let mut scaler = Scaler::get(
        src_fmt,
        src_w,
        src_h,
        Pixel::RGBA,
        src_w,
        src_h,
        Flags::BILINEAR,
    )?;

    let pixel_bytes = (src_w as usize) * (src_h as usize) * 4;
    let mut decoded = Video::empty();
    let mut rgba = Video::empty();

    // Pacing state. `next_wall` is when the *next* frame should be sent;
    // we sleep until then rather than sleeping a per-frame delta, which
    // avoids cumulative drift from kernel scheduler jitter.
    let mut last_pts_us: Option<i64> = None;
    let mut next_wall = Instant::now();

    for (stream, packet) in ictx.packets() {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        if stream.index() != stream_index {
            continue;
        }
        decoder.send_packet(&packet).ok();

        while decoder.receive_frame(&mut decoded).is_ok() {
            if cancel.load(Ordering::Relaxed) {
                return Ok(());
            }

            let pts_us = decoded
                .pts()
                .map(|p| {
                    let num = time_base.numerator() as i64;
                    let den = (time_base.denominator() as i64).max(1);
                    p.saturating_mul(num).saturating_mul(1_000_000) / den
                })
                .unwrap_or(0);

            // --- Rate-controlled pacing ---
            let elapsed_s = playback.started_at.read().elapsed().as_secs_f64();
            let speed = playback.config.read().speed_at(elapsed_s);

            let stream_dt_us = match last_pts_us {
                Some(prev) if pts_us > prev => (pts_us - prev),
                _ => nominal_frame_us,
            };
            let wall_dt_us = (stream_dt_us as f64 / speed) as i64;
            next_wall += Duration::from_micros(wall_dt_us.max(0) as u64);

            let now = Instant::now();
            if next_wall > now {
                thread::sleep(next_wall - now);
            } else if now.saturating_duration_since(next_wall) > Duration::from_millis(250) {
                // We've fallen far behind (paused, slow disk, big speedup);
                // resync rather than sprinting through a backlog.
                next_wall = now;
            }
            last_pts_us = Some(pts_us);

            let capture_us = unix_us();
            scaler.run(&decoded, &mut rgba)?;

            let stride = rgba.stride(0);
            let plane = rgba.data(0);
            let row_bytes = (src_w as usize) * 4;

            let mut buf = Vec::with_capacity(HEADER_LEN + pixel_bytes);
            buf.extend_from_slice(&(src_w as u32).to_le_bytes());
            buf.extend_from_slice(&(src_h as u32).to_le_bytes());
            buf.extend_from_slice(&pts_us.to_le_bytes());
            buf.extend_from_slice(&capture_us.to_le_bytes());

            let send_us_offset = buf.len();
            buf.extend_from_slice(&0i64.to_le_bytes());
            buf.extend_from_slice(&speed.to_le_bytes());

            if stride == row_bytes {
                buf.extend_from_slice(&plane[..pixel_bytes]);
            } else {
                for y in 0..(src_h as usize) {
                    let row_start = y * stride;
                    buf.extend_from_slice(&plane[row_start..row_start + row_bytes]);
                }
            }

            let send_us = unix_us();
            buf[send_us_offset..send_us_offset + 8].copy_from_slice(&send_us.to_le_bytes());

            if channel.send(FrameMessage(buf)).is_err() {
                return Ok(());
            }
        }
    }

    decoder.send_eof().ok();
    while decoder.receive_frame(&mut decoded).is_ok() {
        // drain
    }
    Ok(())
}
