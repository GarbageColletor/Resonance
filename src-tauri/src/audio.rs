use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use rodio::source::SeekError;
use rodio::{OutputStream, Sink, Source};

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
use crate::subsonic::Song;

const QUEUE_CAP: usize = 44_100 * 2 * 4;
const DRAIN_CHUNK: usize = 4096;

#[derive(Clone, Default, serde::Serialize)]
pub struct PlayerState {
    pub playing: bool,
    pub ended: bool,
    pub position_secs: f64,
    pub duration_secs: f64,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
}

pub enum Cmd {
    Play { url: String, song: Song },
    Pause,
    Resume,
    Toggle,
    SeekTo(f64),
    Stop,
    SetVolume(f32),
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
    let (_stream, handle) = match OutputStream::try_default() {
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

    let mut sink: Option<Sink> = None;
    let mut controls = make_controls(tx_self);
    let mut last_progress_push = Instant::now();

    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(cmd) => handle_cmd(cmd, &client, &handle, &mut sink, &mut controls, &state),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        tick_state(&sink, &state, &mut controls, &mut last_progress_push);
    }
}

fn handle_cmd(
    cmd: Cmd,
    client: &reqwest::blocking::Client,
    handle: &rodio::OutputStreamHandle,
    sink: &mut Option<Sink>,
    controls: &mut Option<MediaControls>,
    state: &Arc<Mutex<PlayerState>>,
) {
    match cmd {
        Cmd::Play { url, song } => match build_source(client, &url) {
            Ok(source) => {
                let new_sink = match Sink::try_new(handle) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("[audio] no pude crear sink: {e}");
                        return;
                    }
                };
                new_sink.append(source);
                new_sink.play();
                *sink = Some(new_sink);

                {
                    let mut st = state.lock().unwrap();
                    st.playing = true;
                    st.ended = false;
                    st.position_secs = 0.0;
                    st.duration_secs = song.duration_secs.unwrap_or(0) as f64;
                    st.title = Some(song.title.clone());
                    st.artist = song.artist.clone();
                    st.album = song.album.clone();
                }

                if let Some(c) = controls.as_mut() {
                    let _ = c.set_metadata(MediaMetadata {
                        title: Some(&song.title),
                        artist: song.artist.as_deref(),
                        album: song.album.as_deref(),
                        cover_url: None,
                        duration: song.duration_secs.map(Duration::from_secs),
                    });
                    let _ = c.set_playback(MediaPlayback::Playing {
                        progress: Some(MediaPosition(Duration::ZERO)),
                    });
                }
            }
            Err(e) => eprintln!("[audio] fallo al abrir {}: {e}", song.title),
        },
        Cmd::Pause => {
            if let Some(s) = sink.as_ref() {
                s.pause();
            }
            set_playing(state, false);
            set_control_playback(controls, state, false);
        }
        Cmd::Resume => {
            if let Some(s) = sink.as_ref() {
                s.play();
            }
            set_playing(state, true);
            set_control_playback(controls, state, true);
        }
        Cmd::Toggle => {
            if let Some(s) = sink.as_ref() {
                let now_playing = if s.is_paused() {
                    s.play();
                    true
                } else {
                    s.pause();
                    false
                };
                set_playing(state, now_playing);
                set_control_playback(controls, state, now_playing);
            }
        }
        Cmd::SeekTo(secs) => {
            if let Some(s) = sink.as_ref() {
                match s.try_seek(Duration::from_secs_f64(secs.max(0.0))) {
                    Ok(()) => state.lock().unwrap().position_secs = secs,
                    Err(e) => eprintln!("[audio] seek fallo: {e}"),
                }
            }
        }
        Cmd::Stop => {
            *sink = None;
            {
                let mut st = state.lock().unwrap();
                *st = PlayerState::default();
            }
            if let Some(c) = controls.as_mut() {
                let _ = c.set_playback(MediaPlayback::Stopped);
            }
        }
        Cmd::SetVolume(v) => {
            if let Some(s) = sink.as_ref() {
                s.set_volume(v.clamp(0.0, 2.0));
            }
        }
    }
}

fn tick_state(
    sink: &Option<Sink>,
    state: &Arc<Mutex<PlayerState>>,
    controls: &mut Option<MediaControls>,
    last_progress_push: &mut Instant,
) {
    let Some(s) = sink.as_ref() else { return };

    let ended = s.empty();
    let pos = s.get_pos().as_secs_f64();
    let paused = s.is_paused();

    {
        let mut st = state.lock().unwrap();
        if ended {
            st.playing = false;
            st.ended = true;
            if st.duration_secs > 0.0 {
                st.position_secs = st.duration_secs;
            }
        } else {
            st.position_secs = pos;
            st.playing = !paused;
            st.ended = false;
        }
    }

    if last_progress_push.elapsed() >= Duration::from_millis(1000) {
        if let Some(c) = controls.as_mut() {
            let progress = Some(MediaPosition(Duration::from_secs_f64(pos)));
            let playback = if ended || paused {
                MediaPlayback::Paused { progress }
            } else {
                MediaPlayback::Playing { progress }
            };
            let _ = c.set_playback(playback);
        }
        *last_progress_push = Instant::now();
    }
}

fn set_playing(state: &Arc<Mutex<PlayerState>>, playing: bool) {
    state.lock().unwrap().playing = playing;
}

fn set_control_playback(
    controls: &mut Option<MediaControls>,
    state: &Arc<Mutex<PlayerState>>,
    playing: bool,
) {
    if let Some(c) = controls.as_mut() {
        let pos = state.lock().unwrap().position_secs;
        let progress = Some(MediaPosition(Duration::from_secs_f64(pos)));
        let playback = if playing {
            MediaPlayback::Playing { progress }
        } else {
            MediaPlayback::Paused { progress }
        };
        let _ = c.set_playback(playback);
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
        if left > 0 {
            Some(left)
        } else {
            None
        }
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
        (Some(frames), Some(sr)) if sr > 0 => Some(Duration::from_secs_f64(frames as f64 / sr as f64)),
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
