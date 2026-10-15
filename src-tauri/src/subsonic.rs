use std::io;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::blocking::Client;

pub struct Subsonic {
    pub base: String,
    user: String,
    password: String,
    pub client: Client,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Song {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub suffix: Option<String>,
    #[serde(default)]
    pub duration_secs: Option<u64>,
    #[serde(default)]
    pub cover_url: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResults {
    pub artists: Vec<Artist>,
    pub albums: Vec<Album>,
    pub songs: Vec<Song>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Artist {
    pub id: String,
    pub name: String,
    pub album_count: Option<u64>,
    pub cover_url: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Album {
    pub id: String,
    pub name: String,
    pub artist: Option<String>,
    pub year: Option<u64>,
    pub song_count: Option<u64>,
    pub cover_url: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]

pub struct Playlist {
    pub id: String,
    pub name: String,
    pub song_count: Option<u64>,
    pub cover_url: Option<String>,
}

impl Subsonic {
    pub fn new(base_url: &str, username: &str, password: &str) -> io::Result<Self> {
        let client = Client::builder()
            .user_agent("resonance/0.1")
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(to_io)?;
        Ok(Self {
            base: base_url.trim_end_matches('/').to_string(),
            user: username.to_string(),
            password: password.to_string(),
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

    pub fn random_song(&self) -> io::Result<Song> {
        let v = self.get_json("getRandomSongs", "size=1")?;
        let s = &v["subsonic-response"]["randomSongs"]["song"][0];
        if s.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "getRandomSongs no devolvio canciones",
            ));
        }
        Ok(self.parse_song(s))
    }

    pub fn artists_full(&self) -> io::Result<Vec<Artist>> {
        let v = self.get_json("getArtists", "")?;
        let mut out = Vec::new();
        if let Some(index) = v["subsonic-response"]["artists"]["index"].as_array() {
            for idx in index {
                if let Some(arr) = idx["artist"].as_array() {
                    out.extend(arr.iter().filter_map(|a| self.parse_artist(a)));
                }
            }
        }
        Ok(out)
    }

    pub fn artist_albums(&self, id: &str) -> io::Result<Vec<Album>> {
        let v = self.get_json("getArtist", &format!("id={}", url_encode(id)))?;
        let mut out = Vec::new();
        if let Some(arr) = v["subsonic-response"]["artist"]["album"].as_array() {
            out.extend(arr.iter().filter_map(|a| self.parse_album(a)));
        }
        Ok(out)
    }

    pub fn album_tracks(&self, id: &str) -> io::Result<Vec<Song>> {
        let v = self.get_json("getAlbum", &format!("id={}", url_encode(id)))?;
        let mut out = Vec::new();
        if let Some(arr) = v["subsonic-response"]["album"]["song"].as_array() {
            for s in arr {
                out.push(self.parse_song(s));
            }
        }
        Ok(out)
    }

    pub fn search(&self, query: &str) -> io::Result<SearchResults> {
        let extra = format!(
            "query={}&artistCount=20&albumCount=20&songCount=50",
            url_encode(query)
        );
        let v = self.get_json("search3", &extra)?;
        let r = &v["subsonic-response"]["searchResult3"];
        let artists = r["artist"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| self.parse_artist(x)).collect())
            .unwrap_or_default();
        let albums = r["album"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| self.parse_album(x)).collect())
            .unwrap_or_default();
        let songs = r["song"]
            .as_array()
            .map(|a| a.iter().map(|x| self.parse_song(x)).collect())
            .unwrap_or_default();
        Ok(SearchResults {
            artists,
            albums,
            songs,
        })
    }

    pub fn playlists(&self) -> io::Result<Vec<Playlist>> {
        let v = self.get_json("getPlaylists", "")?;
        let mut out = Vec::new();
        if let Some(arr) = v["subsonic-response"]["playlists"]["playlist"].as_array() {
            for p in arr {
                if let Some(id) = p["id"].as_str() {
                    out.push(Playlist {
                        id: id.to_string(),
                        name: p["name"].as_str().unwrap_or("(sin nombre)").to_string(),
                        song_count: p["songCount"].as_u64(),
                        cover_url: self.cover_url(p),
                    });
                }
            }
        }
        Ok(out)
    }

    pub fn playlist_tracks(&self, id: &str) -> io::Result<Vec<Song>> {
        let v = self.get_json("getPlaylist", &format!("id={}", url_encode(id)))?;
        let mut out = Vec::new();
        if let Some(arr) = v["subsonic-response"]["playlist"]["entry"].as_array() {
            for s in arr {
                out.push(self.parse_song(s));
            }
        }
        Ok(out)
    }

    pub fn starred_songs(&self) -> io::Result<Vec<Song>> {
        let v = self.get_json("getStarred2", "")?;
        let mut out = Vec::new();
        if let Some(arr) = v["subsonic-response"]["starred2"]["song"].as_array() {
            for s in arr {
                out.push(self.parse_song(s));
            }
        }
        Ok(out)
    }

    pub fn stream_url(&self, id: &str) -> String {
        self.rest_url("stream", &format!("id={}", url_encode(id)))
    }

    fn cover_url(&self, v: &serde_json::Value) -> Option<String> {
        v["coverArt"].as_str().map(|cid| {
            self.rest_url("getCoverArt", &format!("id={}&size=256", url_encode(cid)))
        })
    }

    fn parse_artist(&self, a: &serde_json::Value) -> Option<Artist> {
        let id = a["id"].as_str()?;
        Some(Artist {
            id: id.to_string(),
            name: a["name"].as_str().unwrap_or("(sin nombre)").to_string(),
            album_count: a["albumCount"].as_u64(),
            cover_url: self.cover_url(a),
        })
    }

    fn parse_album(&self, a: &serde_json::Value) -> Option<Album> {
        let id = a["id"].as_str()?;
        Some(Album {
            id: id.to_string(),
            name: a["name"].as_str().unwrap_or("(sin titulo)").to_string(),
            artist: a["artist"].as_str().map(str::to_string),
            year: a["year"].as_u64(),
            song_count: a["songCount"].as_u64(),
            cover_url: self.cover_url(a),
        })
    }

    fn parse_song(&self, s: &serde_json::Value) -> Song {
        Song {
            id: s["id"].as_str().unwrap_or("").to_string(),
            title: s["title"].as_str().unwrap_or("(sin titulo)").to_string(),
            artist: s["artist"].as_str().map(str::to_string),
            album: s["album"].as_str().map(str::to_string),
            suffix: s["suffix"].as_str().map(str::to_string),
            duration_secs: s["duration"].as_u64(),
            cover_url: self.cover_url(s),
        }
    }
}

fn to_io<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, scrub(&e.to_string()))
}

pub fn scrub(s: &str) -> String {
    let mut out = s.to_string();
    for base in ["u", "t", "s", "p", "apiKey", "password"] {
        for pre in ['?', '&'] {
            out = redact_param(&out, &format!("{pre}{base}="));
        }
    }
    out
}

fn redact_param(s: &str, key: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find(key) {
        result.push_str(&rest[..pos + key.len()]);
        let after = &rest[pos + key.len()..];
        let end = after
            .find(|c: char| matches!(c, '&' | ' ' | ')' | '"' | '\''))
            .unwrap_or(after.len());
        result.push_str("REDACTED");
        rest = &after[end..];
    }
    result.push_str(rest);
    result
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
