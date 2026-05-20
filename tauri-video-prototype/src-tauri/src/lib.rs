mod video;

use std::sync::Arc;
use std::time::Instant;

use parking_lot::{Mutex, RwLock};
use tauri::ipc::Channel;

pub use video::PlaybackConfig;

/// Shared playback clock: the sine wave phase is computed from
/// `started_at.elapsed()`. Resetting `started_at` when the config changes
/// makes the wave restart at phase 0 each time the user picks a new mode.
pub struct Playback {
    pub config: RwLock<PlaybackConfig>,
    pub started_at: RwLock<Instant>,
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            config: RwLock::new(PlaybackConfig::default()),
            started_at: RwLock::new(Instant::now()),
        }
    }
}

#[derive(Default)]
struct StreamState {
    handle: Mutex<Option<video::DecoderHandle>>,
    playback: Arc<Playback>,
}

#[tauri::command]
async fn start_stream(
    state: tauri::State<'_, Arc<StreamState>>,
    channel: Channel<video::FrameMessage>,
) -> Result<(), String> {
    let path = std::env::var("VIDEO_PATH").map_err(|_| {
        "set VIDEO_PATH=/path/to/file or a URL ffmpeg can open".to_string()
    })?;

    let mut slot = state.handle.lock();
    if let Some(prev) = slot.take() {
        prev.stop();
    }

    let handle =
        video::spawn_decoder(path, channel, state.playback.clone()).map_err(|e| e.to_string())?;
    *slot = Some(handle);
    Ok(())
}

#[tauri::command]
async fn stop_stream(state: tauri::State<'_, Arc<StreamState>>) -> Result<(), String> {
    if let Some(prev) = state.handle.lock().take() {
        prev.stop();
    }
    Ok(())
}

#[tauri::command]
fn set_playback(
    state: tauri::State<'_, Arc<StreamState>>,
    config: PlaybackConfig,
) -> Result<(), String> {
    *state.playback.config.write() = config.sanitized();
    *state.playback.started_at.write() = Instant::now();
    Ok(())
}

pub fn run() {
    ffmpeg_next::init().expect("ffmpeg init");

    tauri::Builder::default()
        .manage(Arc::new(StreamState::default()))
        .invoke_handler(tauri::generate_handler![
            start_stream,
            stop_stream,
            set_playback
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
