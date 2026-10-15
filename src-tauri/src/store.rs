use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const SERVICE: &str = "resonance";

#[derive(Serialize, Deserialize, Clone)]
pub struct SavedServer {
    pub url: String,
    pub username: String,
}

fn data_file(name: &str) -> Option<PathBuf> {
    let mut p = dirs::config_dir()?;
    p.push("resonance");
    Some(p.join(name))
}

fn config_file() -> Option<PathBuf> {
    data_file("server.json")
}

pub fn save_blob(name: &str, bytes: &[u8]) -> io::Result<()> {
    let path = data_file(name)
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "sin directorio de configuracion"))?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, bytes)
}

pub fn load_blob(name: &str) -> Option<Vec<u8>> {
    fs::read(data_file(name)?).ok()
}

pub fn clear_blob(name: &str) {
    if let Some(path) = data_file(name) {
        let _ = fs::remove_file(path);
    }
}

pub fn load_server() -> Option<SavedServer> {
    let data = fs::read(config_file()?).ok()?;
    serde_json::from_slice(&data).ok()
}

pub fn save_server(url: &str, username: &str) -> io::Result<()> {
    let path = config_file()
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "sin directorio de configuracion"))?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let saved = SavedServer {
        url: url.to_string(),
        username: username.to_string(),
    };
    let json = serde_json::to_vec_pretty(&saved)?;
    fs::write(path, json)
}

pub fn clear_server() -> io::Result<()> {
    if let Some(path) = config_file() {
        if path.exists() {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

pub fn save_password(username: &str, password: &str) -> Result<(), String> {
    keyring::Entry::new(SERVICE, username)
        .map_err(|e| e.to_string())?
        .set_password(password)
        .map_err(|e| e.to_string())
}

pub fn load_password(username: &str) -> Option<String> {
    keyring::Entry::new(SERVICE, username).ok()?.get_password().ok()
}

pub fn clear_password(username: &str) {
    if let Ok(entry) = keyring::Entry::new(SERVICE, username) {
        let _ = entry.delete_credential();
    }
}
