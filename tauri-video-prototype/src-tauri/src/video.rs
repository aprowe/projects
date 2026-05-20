//! ffmpeg-next based decoder that pushes RGBA frames through a Tauri
//! `Channel` as raw binary IPC messages.
//!
//! Wire format of each message (little-endian, no padding):
//!   [u32 width][u32 height][i64 pts_us][u8 * width * height * 4 rgba]
//!
//! The frontend pulls width/height out of the first 16 bytes and uses the
//! rest as the `ImageData` pixel buffer. No JSON, no base64 in the hot path.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use anyhow::{anyhow, Context, Result};
use ffmpeg_next as ffmpeg;
use ffmpeg::format::{input, Pixel};
use ffmpeg::media::Type;
use ffmpeg::software::scaling::{context::Context as Scaler, flag::Flags};
use ffmpeg::util::frame::video::Video;
use tauri::ipc::{Channel, InvokeResponseBody, IpcResponse};

/// Binary message pushed through the Tauri channel. We implement
/// `IpcResponse` so Tauri ships it as `InvokeResponseBody::Raw`, which the
/// JS side receives as an `ArrayBuffer` without JSON encoding.
pub struct FrameMessage(pub Vec<u8>);

impl IpcResponse for FrameMessage {
    fn body(self) -> tauri::Result<InvokeResponseBody> {
        Ok(InvokeResponseBody::Raw(self.0))
    }
}

const HEADER_LEN: usize = 4 + 4 + 8; // width + height + pts_us

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

pub fn spawn_decoder(path: String, channel: Channel<FrameMessage>) -> Result<DecoderHandle> {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_thread = cancel.clone();

    let join = thread::Builder::new()
        .name("video-decoder".into())
        .spawn(move || {
            if let Err(e) = decode_loop(&path, &channel, &cancel_thread) {
                eprintln!("decoder error: {e:#}");
            }
        })
        .context("spawning decoder thread")?;

    Ok(DecoderHandle {
        cancel,
        join: Some(join),
    })
}

fn decode_loop(path: &str, channel: &Channel<FrameMessage>, cancel: &AtomicBool) -> Result<()> {
    let mut ictx = input(&path).with_context(|| format!("opening {path}"))?;

    let stream = ictx
        .streams()
        .best(Type::Video)
        .ok_or_else(|| anyhow!("no video stream"))?;
    let stream_index = stream.index();
    let time_base = stream.time_base();

    let context_decoder =
        ffmpeg::codec::context::Context::from_parameters(stream.parameters())?;
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
            scaler.run(&decoded, &mut rgba)?;

            // RGBA plane may be padded per-row; copy line-by-line so the
            // frontend ImageData has no stride gaps.
            let stride = rgba.stride(0);
            let plane = rgba.data(0);
            let row_bytes = (src_w as usize) * 4;

            let mut buf = Vec::with_capacity(HEADER_LEN + pixel_bytes);
            buf.extend_from_slice(&(src_w as u32).to_le_bytes());
            buf.extend_from_slice(&(src_h as u32).to_le_bytes());
            let pts_us = decoded
                .pts()
                .map(|p| {
                    let num = time_base.numerator() as i64;
                    let den = (time_base.denominator() as i64).max(1);
                    p.saturating_mul(num).saturating_mul(1_000_000) / den
                })
                .unwrap_or(0);
            buf.extend_from_slice(&pts_us.to_le_bytes());

            if stride == row_bytes {
                buf.extend_from_slice(&plane[..pixel_bytes]);
            } else {
                for y in 0..(src_h as usize) {
                    let row_start = y * stride;
                    buf.extend_from_slice(&plane[row_start..row_start + row_bytes]);
                }
            }

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
