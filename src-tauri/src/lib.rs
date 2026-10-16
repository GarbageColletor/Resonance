pub mod audio;
pub mod http_source;
pub mod store;
pub mod subsonic;

use std::sync::Mutex;

use audio::{AudioHandle, Cmd, Playhead, PlayerState, SessionSnapshot};
use subsonic::{Album, Artist, Playlist, SearchResults, Song, Subsonic};

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

#[derive(serde::Serialize)]
struct SavedServerInfo {
    url: String,
    username: String,
}

fn classify_error(e: std::io::Error) -> String {
    let low = e.to_string().to_lowercase();
    if low.contains("subsonic error") {
        "El servidor rechazo las credenciales".to_string()
    } else if low.contains("certificate") || low.contains("tls") || low.contains("ssl") {
        "Error de certificado TLS del servidor".to_string()
    } else if low.contains("dns")
        || low.contains("connect")
        || low.contains("tim")
        || low.contains("resolve")
    {
        "No pude conectar con el servidor (revisa la URL y la red)".to_string()
    } else {
        e.to_string()
    }
}

fn restore_session(sub: &Subsonic, audio: &AudioHandle) {
    let Some(bytes) = store::load_blob("session.json") else {
        return;
    };
    let Ok(snap) = serde_json::from_slice::<SessionSnapshot>(&bytes) else {
        return;
    };
    if snap.songs.is_empty() {
        return;
    }
    let items: Vec<(String, Song)> = snap
        .songs
        .iter()
        .map(|s| (sub.stream_url(&s.id), s.clone()))
        .collect();
    let (mut index, mut position_secs, mut volume) = (snap.index, snap.position_secs, snap.volume);
    if let Some(hbytes) = store::load_blob("playhead.json") {
        if let Ok(head) = serde_json::from_slice::<Playhead>(&hbytes) {
            index = head.index;
            position_secs = head.position_secs;
            if head.volume > 0.0 {
                volume = head.volume;
            }
        }
    }
    let index = index.min(items.len() - 1);
    if volume > 0.0 {
        audio.send(Cmd::SetVolume(volume));
    }
    audio.send(Cmd::LoadQueuePaused {
        items,
        index,
        position_secs,
    });
}

fn open(sub: Subsonic) -> Result<(Subsonic, ConnectInfo), String> {
    let version = sub.ping().map_err(classify_error)?;
    let artists = sub.artists_full().map_err(|e| e.to_string())?;
    let info = ConnectInfo {
        server: sub.base.clone(),
        version,
        artist_count: artists.len(),
    };
    Ok((sub, info))
}

#[tauri::command]
fn saved_server() -> Option<SavedServerInfo> {
    store::load_server().map(|s| SavedServerInfo {
        url: s.url,
        username: s.username,
    })
}

#[tauri::command]
fn connect_server(
    state: tauri::State<AppState>,
    url: String,
    username: String,
    password: String,
    remember: bool,
) -> Result<ConnectInfo, String> {
    let sub = Subsonic::new(&url, &username, &password).map_err(|e| e.to_string())?;
    let (sub, info) = open(sub)?;
    if remember {
        store::save_server(&info.server, &username).map_err(|e| e.to_string())?;
        if let Err(e) = store::save_password(&username, &password) {
            eprintln!("[store] no pude guardar la contrasena en el llavero: {e}");
        }
    }
    *state.sub.lock().unwrap() = Some(sub);
    Ok(info)
}

#[tauri::command]
fn connect_saved(state: tauri::State<AppState>) -> Result<ConnectInfo, String> {
    let saved = store::load_server().ok_or("no hay servidor guardado")?;
    let password =
        store::load_password(&saved.username).ok_or("no hay contrasena guardada en el llavero")?;
    let sub = Subsonic::new(&saved.url, &saved.username, &password).map_err(|e| e.to_string())?;
    let (sub, info) = open(sub)?;
    restore_session(&sub, &state.audio);
    *state.sub.lock().unwrap() = Some(sub);
    Ok(info)
}

#[tauri::command]
fn forget_server(state: tauri::State<AppState>) -> Result<(), String> {
    if let Some(s) = store::load_server() {
        store::clear_password(&s.username);
    }
    store::clear_server().map_err(|e| e.to_string())?;
    state.audio.send(Cmd::Stop);
    *state.sub.lock().unwrap() = None;
    Ok(())
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
fn list_playlists(state: tauri::State<AppState>) -> Result<Vec<Playlist>, String> {
    let guard = state.sub.lock().unwrap();
    let sub = guard.as_ref().ok_or("conectate primero")?;
    sub.playlists().map_err(|e| e.to_string())
}

#[tauri::command]
fn playlist_tracks(state: tauri::State<AppState>, playlist_id: String) -> Result<Vec<Song>, String> {
    let guard = state.sub.lock().unwrap();
    let sub = guard.as_ref().ok_or("conectate primero")?;
    sub.playlist_tracks(&playlist_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn list_starred(state: tauri::State<AppState>) -> Result<Vec<Song>, String> {
    let guard = state.sub.lock().unwrap();
    let sub = guard.as_ref().ok_or("conectate primero")?;
    sub.starred_songs().map_err(|e| e.to_string())
}

#[tauri::command]
fn set_star(state: tauri::State<AppState>, id: String, starred: bool) -> Result<(), String> {
    let guard = state.sub.lock().unwrap();
    let sub = guard.as_ref().ok_or("conectate primero")?;
    sub.set_star(&id, starred).map_err(|e| e.to_string())
}

#[tauri::command]
fn search(state: tauri::State<AppState>, query: String) -> Result<SearchResults, String> {
    let guard = state.sub.lock().unwrap();
    let sub = guard.as_ref().ok_or("conectate primero")?;
    sub.search(&query).map_err(|e| e.to_string())
}

#[tauri::command]
fn play_songs(
    state: tauri::State<AppState>,
    songs: Vec<Song>,
    start: usize,
) -> Result<(), String> {
    if songs.is_empty() {
        return Err("sin pistas".into());
    }
    let guard = state.sub.lock().unwrap();
    let sub = guard.as_ref().ok_or("conectate primero")?;
    let items: Vec<(String, Song)> = songs
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
fn toggle_shuffle(state: tauri::State<AppState>) {
    state.audio.send(Cmd::ToggleShuffle);
}

#[tauri::command]
fn cycle_repeat(state: tauri::State<AppState>) {
    state.audio.send(Cmd::CycleRepeat);
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
            saved_server,
            connect_server,
            connect_saved,
            forget_server,
            list_artists,
            list_albums,
            list_album_tracks,
            list_playlists,
            playlist_tracks,
            list_starred,
            set_star,
            search,
            play_album,
            play_songs,
            play_random,
            next_track,
            prev_track,
            replay,
            toggle_play,
            seek,
            set_volume,
            toggle_shuffle,
            cycle_repeat,
            player_state
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
