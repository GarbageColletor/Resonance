use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rodio::source::SeekError;
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};

use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

use crate::http_source::HttpStream;
use crate::store;
use crate::subsonic::Song;

const QUEUE_CAP: usize = 44_100 * 2 * 4;
const DRAIN_CHUNK: usize = 4096;
const SESSION_FILE: &str = "session.json";
const PLAYHEAD_FILE: &str = "playhead.json";

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
pub struct SessionSnapshot {
    pub songs: Vec<Song>,
    pub index: usize,
    pub position_secs: f64,
    pub volume: f32,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
pub struct Playhead {
    pub index: usize,
    pub position_secs: f64,
}

#[derive(Clone, Copy, PartialEq)]
enum RepeatMode {
    Off,
    All,
    One,
}

impl RepeatMode {
    fn as_str(self) -> &'static str {
        match self {
            RepeatMode::Off => "off",
            RepeatMode::All => "all",
            RepeatMode::One => "one",
        }
    }

    fn next(self) -> Self {
        match self {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        }
    }
}

#[derive(Clone, Default, serde::Serialize)]
pub struct PlayerState {
    pub playing: bool,
    pub ended: bool,
    pub has_next: bool,
    pub has_prev: bool,
    pub shuffle: bool,
    pub repeat: String,
    pub position_secs: f64,
    pub duration_secs: f64,
    pub current_id: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub cover_url: Option<String>,
}

pub enum Cmd {
    PlayQueue {
        items: Vec<(String, Song)>,
        start: usize,
    },
    LoadQueuePaused {
        items: Vec<(String, Song)>,
        index: usize,
        position_secs: f64,
    },
    Next,
    Prev,
    Replay,
    Pause,
    Resume,
    Toggle,
    SeekTo(f64),
    Stop,
    SetVolume(f32),
    ToggleShuffle,
    CycleRepeat,
}

#[derive(Clone)]
pub struct AudioHandle {
    tx: Sender<Cmd>,
    state: Arc<Mutex<PlayerState>>,
}

impl AudioHandle {
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(PlayerState::default()));
        let tx_for_media = tx.clone();
        let state_for_thread = Arc::clone(&state);
        std::thread::Builder::new()
            .name("resonance-audio".into())
            .spawn(move || run_audio(rx, tx_for_media, state_for_thread))
            .expect("no pude crear el hilo de audio");
        Self { tx, state }
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }

    pub fn snapshot(&self) -> PlayerState {
        self.state.lock().unwrap().clone()
    }
}

fn run_audio(rx: Receiver<Cmd>, tx_self: Sender<Cmd>, state: Arc<Mutex<PlayerState>>) {
    let (stream, handle) = match OutputStream::try_default() {
        Ok(x) => x,
        Err(e) => {
            eprintln!("[audio] sin dispositivo de salida: {e}");
            return;
        }
    };

    let client = reqwest::blocking::Client::builder()
        .user_agent("resonance/0.1")
        .build()
        .expect("http client");

    let mut engine = Engine {
        _stream: Some(stream),
        handle: Some(handle),
        client,
        state,
        controls: make_controls(tx_self),
        sink: None,
        queue: Vec::new(),
        order: Vec::new(),
        cursor: 0,
        index: 0,
        shuffle: false,
        repeat: RepeatMode::Off,
        volume: 1.0,
        last_push: Instant::now(),
        last_save: Instant::now(),
        last_pos: 0.0,
        frozen_ticks: 0,
        last_recover: Instant::now(),
    };

    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(cmd) => engine.handle(cmd),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        engine.tick();
    }
}

struct Engine {
    _stream: Option<OutputStream>,
    handle: Option<OutputStreamHandle>,
    client: reqwest::blocking::Client,
    state: Arc<Mutex<PlayerState>>,
    controls: Option<MediaControls>,
    sink: Option<Sink>,
    queue: Vec<(String, Song)>,
    order: Vec<usize>,
    cursor: usize,
    index: usize,
    shuffle: bool,
    repeat: RepeatMode,
    volume: f32,
    last_push: Instant,
    last_save: Instant,
    last_pos: f64,
    frozen_ticks: u32,
    last_recover: Instant,
}

impl Engine {
    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::PlayQueue { items, start } => {
                if items.is_empty() {
                    return;
                }
                self.queue = items;
                let start = start.min(self.queue.len() - 1);
                self.build_order(start);
                self.play_cursor(self.cursor, true, 0.0);
                self.save_queue();
            }
            Cmd::LoadQueuePaused {
                items,
                index,
                position_secs,
            } => {
                if items.is_empty() {
                    return;
                }
                self.queue = items;
                let idx = index.min(self.queue.len() - 1);
                self.build_order(idx);
                self.play_cursor(self.cursor, false, position_secs);
                self.save_queue();
            }
            Cmd::Next => {
                if let Some(c) = self.next_cursor() {
                    self.play_cursor(c, true, 0.0);
                }
            }
            Cmd::Prev => {
                if let Some(c) = self.prev_cursor() {
                    self.play_cursor(c, true, 0.0);
                }
            }
            Cmd::Replay => {
                if !self.order.is_empty() {
                    self.play_cursor(self.cursor, true, 0.0);
                }
            }
            Cmd::ToggleShuffle => {
                self.shuffle = !self.shuffle;
                if !self.queue.is_empty() {
                    self.build_order(self.index);
                }
                self.update_nav_state();
                self.save_playhead();
            }
            Cmd::CycleRepeat => {
                self.repeat = self.repeat.next();
                self.update_nav_state();
            }
            Cmd::Pause => {
                if let Some(s) = self.sink.as_ref() {
                    s.pause();
                }
                self.state.lock().unwrap().playing = false;
                self.push_playback(false);
            }
            Cmd::Resume => {
                if let Some(s) = self.sink.as_ref() {
                    s.play();
                }
                self.state.lock().unwrap().playing = true;
                self.push_playback(true);
            }
            Cmd::Toggle => {
                if let Some(s) = self.sink.as_ref() {
                    let now_playing = if s.is_paused() {
                        s.play();
                        true
                    } else {
                        s.pause();
                        false
                    };
                    self.state.lock().unwrap().playing = now_playing;
                    self.push_playback(now_playing);
                }
            }
            Cmd::SeekTo(secs) => {
                if let Some(s) = self.sink.as_ref() {
                    match s.try_seek(Duration::from_secs_f64(secs.max(0.0))) {
                        Ok(()) => self.state.lock().unwrap().position_secs = secs,
                        Err(e) => eprintln!("[audio] seek fallo: {e}"),
                    }
                }
            }
            Cmd::Stop => {
                self.sink = None;
                self.queue.clear();
                self.index = 0;
                *self.state.lock().unwrap() = PlayerState::default();
                store::clear_blob(SESSION_FILE);
                store::clear_blob(PLAYHEAD_FILE);
                if let Some(c) = self.controls.as_mut() {
                    let _ = c.set_playback(MediaPlayback::Stopped);
                }
            }
            Cmd::SetVolume(v) => {
                self.volume = v.clamp(0.0, 2.0);
                if let Some(s) = self.sink.as_ref() {
                    s.set_volume(self.volume);
                }
            }
        }
    }

    fn start_at_opts(&mut self, idx: usize, autoplay: bool, seek_to: f64) {
        if idx >= self.queue.len() {
            return;
        }
        let (url, song) = self.queue[idx].clone();
        let handle = match self.handle.as_ref() {
            Some(h) => h.clone(),
            None => return,
        };
        match build_source(&self.client, &url) {
            Ok(source) => {
                let new_sink = match Sink::try_new(&handle) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("[audio] no pude crear sink: {e}");
                        return;
                    }
                };
                new_sink.set_volume(self.volume);
                new_sink.append(source);
                if autoplay {
                    new_sink.play();
                } else {
                    new_sink.pause();
                }
                if seek_to > 0.0 {
                    let _ = new_sink.try_seek(Duration::from_secs_f64(seek_to));
                }
                self.sink = Some(new_sink);
                self.index = idx;

                {
                    let mut st = self.state.lock().unwrap();
                    st.playing = autoplay;
                    st.ended = false;
                    st.position_secs = seek_to;
                    st.duration_secs = song.duration_secs.unwrap_or(0) as f64;
                    st.current_id = Some(song.id.clone());
                    st.title = Some(song.title.clone());
                    st.artist = song.artist.clone();
                    st.album = song.album.clone();
                    st.cover_url = song.cover_url.clone();
                }
                self.update_nav_state();

                if let Some(c) = self.controls.as_mut() {
                    let _ = c.set_metadata(MediaMetadata {
                        title: Some(&song.title),
                        artist: song.artist.as_deref(),
                        album: song.album.as_deref(),
                        cover_url: None,
                        duration: song.duration_secs.map(Duration::from_secs),
                    });
                    let progress = Some(MediaPosition(Duration::from_secs_f64(seek_to)));
                    let playback = if autoplay {
                        MediaPlayback::Playing { progress }
                    } else {
                        MediaPlayback::Paused { progress }
                    };
                    let _ = c.set_playback(playback);
                }

                self.save_playhead();
            }
            Err(e) => eprintln!("[audio] fallo al abrir {}: {e}", song.title),
        }
    }

    fn save_queue(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        let position_secs = self.state.lock().unwrap().position_secs;
        let snap = SessionSnapshot {
            songs: self.queue.iter().map(|(_, s)| s.clone()).collect(),
            index: self.index,
            position_secs,
            volume: self.volume,
        };
        if let Ok(bytes) = serde_json::to_vec(&snap) {
            let _ = store::save_blob(SESSION_FILE, &bytes);
        }
        self.save_playhead();
    }

    fn save_playhead(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        let position_secs = self.state.lock().unwrap().position_secs;
        let head = Playhead {
            index: self.index,
            position_secs,
        };
        if let Ok(bytes) = serde_json::to_vec(&head) {
            let _ = store::save_blob(PLAYHEAD_FILE, &bytes);
        }
        self.last_save = Instant::now();
    }

    fn build_order(&mut self, start_index: usize) {
        let n = self.queue.len();
        if n == 0 {
            self.order.clear();
            self.cursor = 0;
            return;
        }
        let start = start_index.min(n - 1);
        if self.shuffle {
            let mut rest: Vec<usize> = (0..n).filter(|&i| i != start).collect();
            shuffle_in_place(&mut rest);
            self.order = std::iter::once(start).chain(rest).collect();
            self.cursor = 0;
        } else {
            self.order = (0..n).collect();
            self.cursor = start;
        }
    }

    fn play_cursor(&mut self, cursor: usize, autoplay: bool, seek_to: f64) {
        if cursor >= self.order.len() {
            return;
        }
        self.cursor = cursor;
        let idx = self.order[cursor];
        self.start_at_opts(idx, autoplay, seek_to);
    }

    fn next_cursor(&self) -> Option<usize> {
        if self.order.is_empty() {
            return None;
        }
        if self.cursor + 1 < self.order.len() {
            Some(self.cursor + 1)
        } else if self.repeat == RepeatMode::All {
            Some(0)
        } else {
            None
        }
    }

    fn prev_cursor(&self) -> Option<usize> {
        if self.order.is_empty() {
            return None;
        }
        if self.cursor > 0 {
            Some(self.cursor - 1)
        } else if self.repeat == RepeatMode::All {
            Some(self.order.len() - 1)
        } else {
            None
        }
    }

    fn update_nav_state(&self) {
        let mut st = self.state.lock().unwrap();
        st.has_next = self.cursor + 1 < self.order.len() || self.repeat == RepeatMode::All;
        st.has_prev = self.cursor > 0 || self.repeat == RepeatMode::All;
        st.shuffle = self.shuffle;
        st.repeat = self.repeat.as_str().to_string();
    }

    fn recover_output(&mut self, resume_pos: f64) {
        eprintln!("[audio] salida de audio congelada; recreando OutputStream");
        self.sink = None;
        self._stream = None;
        self.handle = None;
        self.frozen_ticks = 0;
        self.last_recover = Instant::now();
        match OutputStream::try_default() {
            Ok((stream, handle)) => {
                self._stream = Some(stream);
                self.handle = Some(handle);
                if !self.order.is_empty() {
                    self.play_cursor(self.cursor, true, resume_pos);
                }
            }
            Err(e) => eprintln!("[audio] no pude recrear la salida: {e}"),
        }
    }

    fn tick(&mut self) {
        if self.handle.is_none() {
            if !self.order.is_empty() && self.last_recover.elapsed() >= Duration::from_secs(2) {
                let pos = self.state.lock().unwrap().position_secs;
                self.recover_output(pos);
            }
            return;
        }

        let Some(s) = self.sink.as_ref() else {
            return;
        };

        let ended = s.empty();
        let pos = s.get_pos().as_secs_f64();
        let paused = s.is_paused();

        if ended {
            if !self.order.is_empty() {
                if self.repeat == RepeatMode::One {
                    self.play_cursor(self.cursor, true, 0.0);
                    return;
                }
                if let Some(c) = self.next_cursor() {
                    self.play_cursor(c, true, 0.0);
                    return;
                }
            }
            let mut st = self.state.lock().unwrap();
            st.playing = false;
            st.ended = true;
            st.has_next = false;
            if st.duration_secs > 0.0 {
                st.position_secs = st.duration_secs;
            }
            return;
        }

        if !paused {
            if (pos - self.last_pos).abs() < 0.01 {
                self.frozen_ticks += 1;
            } else {
                self.frozen_ticks = 0;
            }
            self.last_pos = pos;
            if self.frozen_ticks >= 6 && self.last_recover.elapsed() >= Duration::from_secs(2) {
                self.frozen_ticks = 0;
                self.recover_output(pos);
                return;
            }
        } else {
            self.frozen_ticks = 0;
            self.last_pos = pos;
        }

        {
            let mut st = self.state.lock().unwrap();
            st.position_secs = pos;
            st.playing = !paused;
            st.ended = false;
        }

        if self.last_push.elapsed() >= Duration::from_millis(1000) {
            if let Some(c) = self.controls.as_mut() {
                let progress = Some(MediaPosition(Duration::from_secs_f64(pos)));
                let playback = if paused {
                    MediaPlayback::Paused { progress }
                } else {
                    MediaPlayback::Playing { progress }
                };
                let _ = c.set_playback(playback);
            }
            self.last_push = Instant::now();
        }

        if self.last_save.elapsed() >= Duration::from_secs(3) {
            self.save_playhead();
        }
    }

    fn push_playback(&mut self, playing: bool) {
        let pos = self.state.lock().unwrap().position_secs;
        if let Some(c) = self.controls.as_mut() {
            let progress = Some(MediaPosition(Duration::from_secs_f64(pos)));
            let playback = if playing {
                MediaPlayback::Playing { progress }
            } else {
                MediaPlayback::Paused { progress }
            };
            let _ = c.set_playback(playback);
        }
    }
}

fn shuffle_in_place(v: &mut [usize]) {
    if v.len() < 2 {
        return;
    }
    let mut state = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9e37_79b9_7f4a_7c15)
        | 1;
    for i in (1..v.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let j = (state % (i as u64 + 1)) as usize;
        v.swap(i, j);
    }
}

fn make_controls(tx: Sender<Cmd>) -> Option<MediaControls> {
    let config = PlatformConfig {
        dbus_name: "resonance",
        display_name: "Resonance",
        hwnd: None,
    };
    let mut controls = match MediaControls::new(config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[audio] controles de sistema no disponibles: {e:?}");
            return None;
        }
    };
    let attached = controls.attach(move |event: MediaControlEvent| {
        let cmd = match event {
            MediaControlEvent::Play => Some(Cmd::Resume),
            MediaControlEvent::Pause => Some(Cmd::Pause),
            MediaControlEvent::Toggle => Some(Cmd::Toggle),
            MediaControlEvent::Next => Some(Cmd::Next),
            MediaControlEvent::Previous => Some(Cmd::Prev),
            MediaControlEvent::Stop => Some(Cmd::Stop),
            MediaControlEvent::SetPosition(MediaPosition(d)) => Some(Cmd::SeekTo(d.as_secs_f64())),
            _ => None,
        };
        if let Some(c) = cmd {
            let _ = tx.send(c);
        }
    });
    if let Err(e) = attached {
        eprintln!("[audio] no pude enganchar los controles de sistema: {e:?}");
    }
    Some(controls)
}

struct QueueInner {
    buf: VecDeque<f32>,
    eof: bool,
    seek_to: Option<f64>,
    stop: bool,
}

struct SampleQueue {
    inner: Mutex<QueueInner>,
    cv: Condvar,
}

impl SampleQueue {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(QueueInner {
                buf: VecDeque::new(),
                eof: false,
                seek_to: None,
                stop: false,
            }),
            cv: Condvar::new(),
        })
    }
}

fn produce(
    queue: Arc<SampleQueue>,
    mut format: Box<dyn FormatReader>,
    mut decoder: Box<dyn Decoder>,
    track_id: u32,
) {
    loop {
        {
            let mut g = queue.inner.lock().unwrap();
            if g.stop {
                return;
            }
            if let Some(t) = g.seek_to.take() {
                drop(g);
                let _ = format.seek(
                    SeekMode::Accurate,
                    SeekTo::Time {
                        time: Time::from(t),
                        track_id: Some(track_id),
                    },
                );
                decoder.reset();
                let mut g2 = queue.inner.lock().unwrap();
                g2.buf.clear();
                g2.eof = false;
                continue;
            }
        }

        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(_) => {
                park_until_seek_or_stop(&queue);
                continue;
            }
        };
        if packet.track_id() != track_id {
            continue;
        }

        let samples = match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                let cap = decoded.capacity() as u64;
                let mut sb = SampleBuffer::<f32>::new(cap, spec);
                sb.copy_interleaved_ref(decoded);
                sb.samples().to_vec()
            }
            Err(SymError::DecodeError(_)) => continue,
            Err(_) => {
                park_until_seek_or_stop(&queue);
                continue;
            }
        };

        let mut g = queue.inner.lock().unwrap();
        loop {
            if g.stop {
                return;
            }
            if g.seek_to.is_some() {
                break;
            }
            if g.buf.len() + samples.len() <= QUEUE_CAP {
                break;
            }
            g = queue.cv.wait(g).unwrap();
        }
        if g.stop {
            return;
        }
        if g.seek_to.is_some() {
            continue;
        }
        g.buf.extend(samples);
        queue.cv.notify_all();
    }
}

fn park_until_seek_or_stop(queue: &Arc<SampleQueue>) {
    let mut g = queue.inner.lock().unwrap();
    g.eof = true;
    while g.seek_to.is_none() && !g.stop {
        g = queue.cv.wait(g).unwrap();
    }
}

struct StreamingSource {
    queue: Arc<SampleQueue>,
    local: Vec<f32>,
    lpos: usize,
    sample_rate: u32,
    channels: u16,
    total: Option<Duration>,
    ended: bool,
}

impl Iterator for StreamingSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.lpos < self.local.len() {
            let s = self.local[self.lpos];
            self.lpos += 1;
            return Some(s);
        }
        if self.ended {
            return None;
        }

        let mut g = self.queue.inner.lock().unwrap();
        let n = DRAIN_CHUNK.min(g.buf.len());
        if n == 0 {
            if g.eof {
                drop(g);
                self.ended = true;
                return None;
            }
            drop(g);
            self.local = vec![0.0; DRAIN_CHUNK];
            self.lpos = 1;
            return Some(0.0);
        }
        self.local = g.buf.drain(..n).collect();
        drop(g);
        self.queue.cv.notify_all();
        self.lpos = 1;
        Some(self.local[0])
    }
}

impl Source for StreamingSource {
    fn current_frame_len(&self) -> Option<usize> {
        let left = self.local.len().saturating_sub(self.lpos);
        if left > 0 { Some(left) } else { None }
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        self.total
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        {
            let mut g = self.queue.inner.lock().unwrap();
            g.buf.clear();
            g.seek_to = Some(pos.as_secs_f64());
            g.eof = false;
        }
        self.queue.cv.notify_all();
        self.local.clear();
        self.lpos = 0;
        self.ended = false;
        Ok(())
    }
}

impl Drop for StreamingSource {
    fn drop(&mut self) {
        {
            let mut g = self.queue.inner.lock().unwrap();
            g.stop = true;
        }
        self.queue.cv.notify_all();
    }
}

fn build_source(client: &reqwest::blocking::Client, url: &str) -> Result<StreamingSource, String> {
    let stream = HttpStream::new(client.clone(), url.to_string()).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(stream), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("flac");

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("probe: {e}"))?;
    let format = probed.format;

    let track = format.default_track().ok_or("sin pista de audio")?;
    let track_id = track.id;
    let params = track.codec_params.clone();

    let sample_rate = params.sample_rate.unwrap_or(44_100);
    let channels = params.channels.map(|c| c.count() as u16).unwrap_or(2);
    let total = match (params.n_frames, params.sample_rate) {
        (Some(frames), Some(sr)) if sr > 0 => {
            Some(Duration::from_secs_f64(frames as f64 / sr as f64))
        }
        _ => None,
    };

    let decoder = symphonia::default::get_codecs()
        .make(&params, &DecoderOptions::default())
        .map_err(|e| format!("decoder: {e}"))?;

    let queue = SampleQueue::new();
    let producer_queue = Arc::clone(&queue);
    std::thread::Builder::new()
        .name("resonance-decode".into())
        .spawn(move || produce(producer_queue, format, decoder, track_id))
        .map_err(|e| e.to_string())?;

    Ok(StreamingSource {
        queue,
        local: Vec::new(),
        lpos: 0,
        sample_rate,
        channels,
        total,
        ended: false,
    })
}
