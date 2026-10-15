# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this project is

**Resonance** (working name) is a desktop-first (Linux and Windows first, macOS/Android/iOS later) music client for self-hosted libraries. It talks directly to a user's own server over the Subsonic/OpenSubsonic protocol, with Navidrome as the first target. There is intentionally no central backend of our own: all traffic goes from the client to the user's music server.

The full design spec lives in `docs/` (written in Spanish) and is the source of truth. Read `docs/README.md` first, then the numbered docs under `docs/docs/` (01 product through 09 open decisions). The docs are the design, not a locked decision log: `docs/docs/09-decisiones.md` separates what is conceptually agreed from what is still open (name, license, Linux packaging, etc.). Treat ADR items as proposals to validate.

## Current state

Working against a real Navidrome: connect with token auth, browse artists then albums then album tracks, play an album as a queue with auto-advance and next/previous, seek over HTTP streaming, replay, play a random track, and OS media keys (including next/previous via MPRIS). The code is still flat (not yet in the doc-04 layout):

- `src/App.tsx`, `src/App.css` : React UI. `App` owns connection and library browsing; `BrowseList` (memoized) renders the artist/album/track list; `PlayerBar` (memoized) owns the player-state poll and transport controls. The two are isolated so the ~250ms poll only re-renders the small player bar, not the list. Albums-by-artist and tracks-by-album are cached; navigation is optimistic (view switches first, shows "Cargando").
- `src-tauri/src/subsonic.rs` : OpenSubsonic client (token auth, ping, getArtists, getArtist, getAlbum, getRandomSongs, stream URL).
- `src-tauri/src/http_source.rs` : `HttpStream`, a `Read + Seek` source over HTTP range requests, also a Symphonia `MediaSource`.
- `src-tauri/src/audio.rs` : native audio service. `Engine` owns the play queue (Vec of (url, Song) plus index) with auto-advance and next/previous, runs on its own thread with a command queue; `SampleQueue` plus a producer thread decode ahead into a bounded ring buffer that the Rodio `Source` drains without blocking; `souvlaki` for media keys.
- `src-tauri/src/lib.rs` : Tauri state and commands (connect, list_artists, list_albums, list_album_tracks, play_album, play_random, next_track, prev_track, replay, toggle_play, seek, set_volume, player_state).

Stack in use: Tauri 2, React 19, TypeScript, Vite, Rust. Audio is Symphonia + Rodio + CPAL + souvlaki. Stream URLs carry Subsonic token auth and do not expire within the session, so a whole album's URLs are prebuilt in the command layer and handed to the audio thread, which advances the queue without needing credentials. SQLite and secure credential storage are planned but not added yet (dev credentials live in `resonance.dev.json`).

## Target architecture (what the code should grow into)

The spec mandates strict separation so the UI knows nothing about Subsonic endpoints or how secrets are stored (`docs/docs/04-arquitectura.md`, `docs/docs/05-tecnico.md`). Once there is more living code, refactor the flat files into:

- `ui/` : views, navigation, accessibility. Never handles plaintext passwords after login.
- `domain/` : entities, queue, playback policies, use cases. The `MusicProvider` and `AudioEngine` interfaces live here.
- `providers/opensubsonic/` : capability detection, data and error mapping. The only layer that knows about `/rest/...` Subsonic calls.
- audio (native, Rust in `src-tauri/`) : playback is a native Rust service, not webview audio. The webview is only UI and visualizers. The domain's `AudioEngine` port is a Tauri command/event bridge to this service; the webview never touches an `HTMLAudioElement` for playback.
- `storage/` : SQLite metadata cache, local stats, preferences, versioned migrations.
- `security/` : OS keychain/vault interaction and URL normalization.

Data flow: UI to use case to `MusicProvider` (port) to OpenSubsonic adapter to HTTP client to server. Audio is a separate port (`AudioEngine`) so DSP, background, and platform differences stay out of the domain.

## Invariants that are easy to get wrong

Deliberate decisions from the spec. Writing the "obvious" code instead is the main hazard here:

- Entity IDs are always composite `(sourceId, remoteId)`, from day one, even though multi-server is out of MVP scope. This keeps the door open for unified libraries later.
- Treat Navidrome/Subsonic IDs as strings, never numbers.
- `PlaybackSource` is not just a URL anywhere in the code. It may carry headers, expiry, transcoding options, and credentials. Keep it an abstraction, and keep sensitive URLs out of logs, history, notifications, and crash reports.
- `stream` alone does not count a play on Navidrome. Use `scrobble` with correct semantics, dedupe across retries, and respect the configurable listen threshold.
- Detect server capabilities before enabling features (`probe()` / `ProviderCapabilities`). The Subsonic ecosystem is heterogeneous; never assume every server implements the same endpoints or auth. OpenSubsonic `apiKey` auth needs a fallback by capability.
- Do not build the MVP on Navidrome's own `/api/v1`; it is still marked unstable upstream. Use the Subsonic-compatible `/rest/...` surface.
- Local `play_events` are device-local, not a mirror of server activity. Keep local statistics distinct from server-side counts; history must be exportable and deletable without touching the server.
- Secrets go in the OS keychain/vault; SQLite stores only references (`secret_ref`), never plaintext credentials.
- HTTPS by default. HTTP (LAN) requires an explicit user warning flow; never silently accept TLS errors.

## Audio findings (Milestone 0)

- Decode with Symphonia directly, not `rodio::Decoder`. Rodio 0.20's `ReadSeekSource` hardcodes `byte_len() = None`, which makes Symphonia report FLAC as `Unseekable`. `HttpStream` wrapped as a Symphonia `MediaSource` returning `byte_len = Some(len)` and `is_seekable = true` seeks correctly. Rodio is used only as the output sink via a custom `Source` (`SymphoniaSource`) whose `try_seek` drives the Symphonia format reader. Decoding ourselves is also what will later give the PCM tap for the FFT visualizer and the EQ/DSP chain.
- Navidrome's `stream` endpoint honors range requests (`Accept-Ranges: bytes`), so no temp-file buffering is needed.
- OS media controls (MPRIS/SMTC/Now Playing) are not provided by Rodio/CPAL; `souvlaki` handles them.
- Server is all-FLAC, so Opus (Symphonia's weak spot) is a non-issue for now.
- Known debt: `SymphoniaSource::next()` does blocking HTTP reads on Rodio's audio output thread, so an empty buffer (notably right after a seek) causes occasional ALSA underruns. The fix is a producer thread that decodes ahead into a bounded ring buffer the Rodio `Source` drains without blocking.

## Conventions

- No comments in code. No emojis anywhere in code or UI strings. No em or en dashes in code or documentation (plain chat is fine).
- Spec language is Spanish; the app must be i18n-ready for Spanish and English (NFR-10).
- Accessibility is P0, not polish: keyboard-navigable, screen-reader friendly, visible focus, honor `prefers-reduced-motion`.
- Design direction ("simple, not minimalist"): muted palette, layered surfaces, a harmonic accent that never replaces labels or hierarchy. Theme tokens (CSS variables like `--bg`, `--surface`, `--accent`) are in `docs/docs/06-diseno.md`, exploratory until WCAG-contrast-checked.
- MVP scope rule: a feature is in only if it directly serves connecting, finding, and listening to music reliably (`docs/docs/07-mvp-roadmap.md`). EQ, visualizers, offline, full stats, and multi-server are deferred.

## Running the app (dev)

- `npm run app` launches the Tauri desktop app (Vite plus Rust). Use this, not `npm run tauri dev`, on Linux: it sets `WEBKIT_DISABLE_DMABUF_RENDERER=1`, without which WebKitGTK crashes on Wayland with `Gdk-Message: Error 71 (protocol error) dispatching to Wayland display`. This is a WebKitGTK/Wayland environment issue, not an app bug.
- First Rust build is slow (Symphonia/CPAL/reqwest); later builds are incremental.
- Dev credentials live in `resonance.dev.json` at the repo root (gitignored). Copy `resonance.dev.example.json` to create it.

## Git

Do not run `git commit`, `git push`, or otherwise write to git or GitHub. The user handles all commits himself. The working branch is `master`; PRs target `main`.
