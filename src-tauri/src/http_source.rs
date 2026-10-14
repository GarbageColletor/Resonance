use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Mutex;

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use reqwest::header::{ACCEPT_RANGES, CONTENT_RANGE, RANGE};

pub struct HttpStream {
    client: Client,
    url: String,
    pos: u64,
    len: u64,
    accepts_ranges: bool,
    reader: Mutex<Option<Response>>,
}

impl HttpStream {
    pub fn new(client: Client, url: String) -> io::Result<Self> {
        let resp = client
            .get(&url)
            .header(RANGE, "bytes=0-")
            .send()
            .map_err(to_io)?;

        let accepts_ranges = resp.status() == StatusCode::PARTIAL_CONTENT
            || resp
                .headers()
                .get(ACCEPT_RANGES)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.contains("bytes"))
                .unwrap_or(false);

        let len = total_len(&resp);

        Ok(Self {
            client,
            url,
            pos: 0,
            len,
            accepts_ranges,
            reader: Mutex::new(Some(resp)),
        })
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn accepts_ranges(&self) -> bool {
        self.accepts_ranges
    }

    fn open_at(&mut self, pos: u64) -> io::Result<()> {
        let resp = self
            .client
            .get(&self.url)
            .header(RANGE, format!("bytes={pos}-"))
            .send()
            .map_err(to_io)?;
        *self.reader.get_mut().unwrap() = Some(resp);
        Ok(())
    }
}

impl Read for HttpStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.reader.get_mut().unwrap().is_none() {
            self.open_at(self.pos)?;
        }
        let n = self.reader.get_mut().unwrap().as_mut().unwrap().read(buf)?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for HttpStream {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let new = match from {
            SeekFrom::Start(n) => n,
            SeekFrom::End(n) => (self.len as i64 + n).max(0) as u64,
            SeekFrom::Current(n) => (self.pos as i64 + n).max(0) as u64,
        };
        if new != self.pos {
            self.pos = new;
            *self.reader.get_mut().unwrap() = None;
        }
        Ok(self.pos)
    }
}

impl symphonia::core::io::MediaSource for HttpStream {
    fn is_seekable(&self) -> bool {
        self.accepts_ranges
    }
    fn byte_len(&self) -> Option<u64> {
        if self.len > 0 { Some(self.len) } else { None }
    }
}

fn total_len(resp: &Response) -> u64 {
    if let Some(cr) = resp
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(total) = cr.rsplit('/').next() {
            if let Ok(n) = total.trim().parse::<u64>() {
                return n;
            }
        }
    }
    resp.content_length().unwrap_or(0)
}

fn to_io<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
}
