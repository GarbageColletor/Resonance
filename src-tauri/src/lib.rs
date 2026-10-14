pub mod audio;
pub mod http_source;
pub mod subsonic;

use std::sync::Mutex;

use audio::{AudioHandle, Cmd, PlayerState};
use subsonic::{Song, Subsonic};

const DEV_CONFIG: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../resonance.dev.json");

struct AppState {
    sub: Mutex<Option<Subsonic>>,
    audio: AudioHandle,
    last: Mutex<Option<(String, Song)>>,
}

#[derive(serde::Serialize)]
struct ConnectInfo {
    server: String,
    version: String,
    artist_count: usize,
    artists_sample: Vec<String>,
}

#[derive(serde::Serialize)]
struct SongDto {
    id: String,
    title: String,
    artist: Option<String>,
    album: Option<String>,
    duration_secs: Option<u64>,
}

#[tauri::command]
fn connect(state: tauri::State<AppState>) -> Result<ConnectInfo, String> {
    let sub = Subsonic::from_dev_config(DEV_CONFIG).map_err(|e| e.to_string())?;
    let version = sub.ping().map_err(|e| e.to_string())?;
    let (artist_count, artists_sample) = sub.artists().map_err(|e| e.to_string())?;
    let server = sub.base.clone();
    *state.sub.lock().unwrap() = Some(sub);
    Ok(ConnectInfo {
        server,
        version,
        artist_count,
        artists_sample,
    })
}

#[tauri::command]
fn play_random(state: tauri::State<AppState>) -> Result<SongDto, String> {
    let guard = state.sub.lock().unwrap();
    let sub = guard.as_ref().ok_or("conéctate primero")?;
    let song = sub.random_song().map_err(|e| e.to_string())?;
    let url = sub.stream_url(&song.id);
    let dto = SongDto {
        id: song.id.clone(),
        title: song.title.clone(),
        artist: song.artist.clone(),
        album: song.album.clone(),
        duration_secs: song.duration_secs,
    };
    *state.last.lock().unwrap() = Some((url.clone(), song.clone()));
    state.audio.send(Cmd::Play { url, song });
    Ok(dto)
}

#[tauri::command]
fn replay(state: tauri::State<AppState>) -> Result<(), String> {
    let (url, song) = state
        .last
        .lock()
        .unwrap()
        .as_ref()
        .ok_or("no hay pista para repetir")?
        .clone();
    state.audio.send(Cmd::Play { url, song });
    Ok(())
}

#[tauri::command]
fn toggle_play(state: tauri::State<AppState>) {
    state.audio.send(Cmd::Toggle);
}

#[tauri::command]
fn seek(state: tauri::State<AppState>, secs: f64) {
    state.audio.send(Cmd::SeekTo(secs));
}

#[tauri::command]
fn set_volume(state: tauri::State<AppState>, volume: f32) {
    state.audio.send(Cmd::SetVolume(volume));
}

#[tauri::command]
fn player_state(state: tauri::State<AppState>) -> PlayerState {
    state.audio.snapshot()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app_state = AppState {
        sub: Mutex::new(None),
        audio: AudioHandle::spawn(),
        last: Mutex::new(None),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            connect,
            play_random,
            replay,
            toggle_play,
            seek,
            set_volume,
            player_state
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
