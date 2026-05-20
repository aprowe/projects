mod video;

use std::sync::Arc;

use parking_lot::Mutex;
use tauri::ipc::Channel;

#[derive(Default)]
struct StreamState {
    handle: Mutex<Option<video::DecoderHandle>>,
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

    let handle = video::spawn_decoder(path, channel).map_err(|e| e.to_string())?;
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

pub fn run() {
    ffmpeg_next::init().expect("ffmpeg init");

    tauri::Builder::default()
        .manage(Arc::new(StreamState::default()))
        .invoke_handler(tauri::generate_handler![start_stream, stop_stream])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
