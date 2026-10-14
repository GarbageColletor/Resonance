use std::io;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::blocking::Client;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct DevConfig {
    #[serde(rename = "serverUrl")]
    pub server_url: String,
    pub username: String,
    pub password: String,
}

pub struct Subsonic {
    pub base: String,
    user: String,
    password: String,
    pub client: Client,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Song {
    pub id: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub suffix: Option<String>,
    pub duration_secs: Option<u64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Artist {
    pub id: String,
    pub name: String,
    pub album_count: Option<u64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Album {
    pub id: String,
    pub name: String,
    pub artist: Option<String>,
    pub year: Option<u64>,
    pub song_count: Option<u64>,
}

impl Subsonic {
    pub fn from_dev_config(path: &str) -> io::Result<Self> {
        let data = std::fs::read(path)?;
        let cfg: DevConfig = serde_json::from_slice(&data)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let client = Client::builder()
            .user_agent("resonance/0.1")
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(to_io)?;
        let base = cfg.server_url.trim_end_matches('/').to_string();
        Ok(Self {
            base,
            user: cfg.username,
            password: cfg.password,
            client,
        })
    }

    fn auth_query(&self) -> String {
        let salt = format!(
            "{:x}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let token = format!("{:x}", md5::compute(format!("{}{}", self.password, salt)));
        format!(
            "u={}&t={}&s={}&v=1.16.1&c=resonance&f=json",
            url_encode(&self.user),
            token,
            salt
        )
    }

    pub fn rest_url(&self, method: &str, extra: &str) -> String {
        let mut u = format!("{}/rest/{}?{}", self.base, method, self.auth_query());
        if !extra.is_empty() {
            u.push('&');
            u.push_str(extra);
        }
        u
    }

    fn get_json(&self, method: &str, extra: &str) -> io::Result<serde_json::Value> {
        let url = self.rest_url(method, extra);
        let resp = self.client.get(&url).send().map_err(to_io)?;
        let v: serde_json::Value = resp.json().map_err(to_io)?;
        let sr = &v["subsonic-response"];
        if sr["status"] != "ok" {
            let msg = sr["error"]["message"]
                .as_str()
                .unwrap_or("error desconocido");
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("subsonic error: {msg}"),
            ));
        }
        Ok(v)
    }

    pub fn ping(&self) -> io::Result<String> {
        let v = self.get_json("ping", "")?;
        Ok(v["subsonic-response"]["version"]
            .as_str()
            .unwrap_or("?")
            .to_string())
    }

    pub fn artists(&self) -> io::Result<(usize, Vec<String>)> {
        let v = self.get_json("getArtists", "")?;
        let mut names = Vec::new();
        if let Some(index) = v["subsonic-response"]["artists"]["index"].as_array() {
            for idx in index {
                if let Some(arr) = idx["artist"].as_array() {
                    for a in arr {
                        if let Some(n) = a["name"].as_str() {
                            names.push(n.to_string());
                        }
                    }
                }
            }
        }
        let count = names.len();
        names.truncate(5);
        Ok((count, names))
    }

    pub fn random_song(&self) -> io::Result<Song> {
        let v = self.get_json("getRandomSongs", "size=1")?;
        let s = &v["subsonic-response"]["randomSongs"]["song"][0];
        if s.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "getRandomSongs no devolvio canciones",
            ));
        }
        Ok(song_from(s))
    }

    pub fn artists_full(&self) -> io::Result<Vec<Artist>> {
        let v = self.get_json("getArtists", "")?;
        let mut out = Vec::new();
        if let Some(index) = v["subsonic-response"]["artists"]["index"].as_array() {
            for idx in index {
                if let Some(arr) = idx["artist"].as_array() {
                    for a in arr {
                        if let Some(id) = a["id"].as_str() {
                            out.push(Artist {
                                id: id.to_string(),
                                name: a["name"].as_str().unwrap_or("(sin nombre)").to_string(),
                                album_count: a["albumCount"].as_u64(),
                            });
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    pub fn artist_albums(&self, id: &str) -> io::Result<Vec<Album>> {
        let v = self.get_json("getArtist", &format!("id={}", url_encode(id)))?;
        let mut out = Vec::new();
        if let Some(arr) = v["subsonic-response"]["artist"]["album"].as_array() {
            for a in arr {
                if let Some(aid) = a["id"].as_str() {
                    out.push(Album {
                        id: aid.to_string(),
                        name: a["name"].as_str().unwrap_or("(sin titulo)").to_string(),
                        artist: a["artist"].as_str().map(str::to_string),
                        year: a["year"].as_u64(),
                        song_count: a["songCount"].as_u64(),
                    });
                }
            }
        }
        Ok(out)
    }

    pub fn album_tracks(&self, id: &str) -> io::Result<Vec<Song>> {
        let v = self.get_json("getAlbum", &format!("id={}", url_encode(id)))?;
        let mut out = Vec::new();
        if let Some(arr) = v["subsonic-response"]["album"]["song"].as_array() {
            for s in arr {
                out.push(song_from(s));
            }
        }
        Ok(out)
    }

    pub fn stream_url(&self, id: &str) -> String {
        self.rest_url("stream", &format!("id={}", url_encode(id)))
    }
}

fn song_from(s: &serde_json::Value) -> Song {
    Song {
        id: s["id"].as_str().unwrap_or("").to_string(),
        title: s["title"].as_str().unwrap_or("(sin titulo)").to_string(),
        artist: s["artist"].as_str().map(str::to_string),
        album: s["album"].as_str().map(str::to_string),
        suffix: s["suffix"].as_str().map(str::to_string),
        duration_secs: s["duration"].as_u64(),
    }
}

fn to_io<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}
