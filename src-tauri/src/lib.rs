pub mod audio;
pub mod http_source;
pub mod subsonic;

use std::sync::Mutex;

use audio::{AudioHandle, Cmd, PlayerState};
use subsonic::{Album, Artist, Song, Subsonic};

const DEV_CONFIG: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../resonance.dev.json");

struct AppState {
    sub: Mutex<Option<Subsonic>>,
    audio: AudioHandle,
}

#[derive(serde::Serialize)]
struct ConnectInfo {
    server: String,
    version: String,
    artist_count: usize,
}

#[tauri::command]
fn connect(state: tauri::State<AppState>) -> Result<ConnectInfo, String> {
    let sub = Subsonic::from_dev_config(DEV_CONFIG).map_err(|e| e.to_string())?;
    let version = sub.ping().map_err(|e| e.to_string())?;
    let artists = sub.artists_full().map_err(|e| e.to_string())?;
    let server = sub.base.clone();
    let artist_count = artists.len();
    *state.sub.lock().unwrap() = Some(sub);
    Ok(ConnectInfo {
        server,
        version,
        artist_count,
    })
}

#[tauri::command]
fn list_artists(state: tauri::State<AppState>) -> Result<Vec<Artist>, String> {
    let guard = state.sub.lock().unwrap();
    let sub = guard.as_ref().ok_or("conectate primero")?;
    sub.artists_full().map_err(|e| e.to_string())
}

#[tauri::command]
fn list_albums(state: tauri::State<AppState>, artist_id: String) -> Result<Vec<Album>, String> {
    let guard = state.sub.lock().unwrap();
    let sub = guard.as_ref().ok_or("conectate primero")?;
    sub.artist_albums(&artist_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn list_album_tracks(state: tauri::State<AppState>, album_id: String) -> Result<Vec<Song>, String> {
    let guard = state.sub.lock().unwrap();
    let sub = guard.as_ref().ok_or("conectate primero")?;
    sub.album_tracks(&album_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn play_album(state: tauri::State<AppState>, album_id: String, start: usize) -> Result<(), String> {
    let guard = state.sub.lock().unwrap();
    let sub = guard.as_ref().ok_or("conectate primero")?;
    let tracks = sub.album_tracks(&album_id).map_err(|e| e.to_string())?;
    if tracks.is_empty() {
        return Err("album sin pistas".into());
    }
    let items: Vec<(String, Song)> = tracks
        .into_iter()
        .map(|s| (sub.stream_url(&s.id), s))
        .collect();
    state.audio.send(Cmd::PlayQueue { items, start });
    Ok(())
}

#[tauri::command]
fn play_random(state: tauri::State<AppState>) -> Result<Song, String> {
    let guard = state.sub.lock().unwrap();
    let sub = guard.as_ref().ok_or("conectate primero")?;
    let song = sub.random_song().map_err(|e| e.to_string())?;
    let url = sub.stream_url(&song.id);
    let out = song.clone();
    state.audio.send(Cmd::PlayQueue {
        items: vec![(url, song)],
        start: 0,
    });
    Ok(out)
}

#[tauri::command]
fn next_track(state: tauri::State<AppState>) {
    state.audio.send(Cmd::Next);
}

#[tauri::command]
fn prev_track(state: tauri::State<AppState>) {
    state.audio.send(Cmd::Prev);
}

#[tauri::command]
fn replay(state: tauri::State<AppState>) {
    state.audio.send(Cmd::Replay);
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
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            connect,
            list_artists,
            list_albums,
            list_album_tracks,
            play_album,
            play_random,
            next_track,
            prev_track,
            replay,
            toggle_play,
            seek,
            set_volume,
            player_state
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
