# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this project is

**Resonance** (working name) is a desktop-first (Linux and Windows first, macOS/Android/iOS later) music client for self-hosted libraries. It talks directly to a user's own server over the Subsonic/OpenSubsonic protocol, with Navidrome as the first target. There is intentionally no central backend of our own: all traffic goes from the client to the user's music server.

The full design spec lives in `docs/` (written in Spanish) and is the source of truth. Read `docs/README.md` first, then the numbered docs under `docs/docs/` (01 product through 09 open decisions). The docs are the design, not a locked decision log: `docs/docs/09-decisiones.md` separates what is conceptually agreed from what is still open (name, license, Linux packaging, etc.). Treat ADR items as proposals to validate.

## Current state

Resonance runs as a real player against a real Navidrome. Implemented:

- Connection wizard: the user enters server URL, username and password in the UI. URL and username are saved to `~/.config/resonance/server.json`; the password goes to the OS keychain (Secret Service via the `keyring` crate), never to disk in plaintext (NFR-03). On launch the app auto-reconnects from the saved server; "Cambiar servidor" forgets it. Connection errors are classified (bad credentials vs network vs TLS).
- Library: browse artists then albums then album tracks; search (search3) across artists/albums/songs; playlists (getPlaylists/getPlaylist) and favorites (getStarred2) as top-level tabs; cover art (getCoverArt) in lists and the player bar.
- Playback: native audio queue with auto-advance, next/previous, replay, seek over HTTP streaming, play album / play a song list / play random. OS media keys via MPRIS including next/previous.
- Session persistence (FR-09): the engine saves the queue (songs and ids, no stream URLs) plus a small playhead (index + position) to the config dir; on reconnect the queue is restored paused at the saved position, with the token-bearing URLs rebuilt fresh. Password and stream tokens are never written to disk.

The code is still flat (not yet in the doc-04 layout):

- `src/App.tsx`, `src/App.css` : React UI. `App` owns connection, the connection form, library browsing and the tab switcher (Artistas / Playlists / Favoritos). `BrowseList`, `SearchResultsView`, `SongList` and `Cover` are separate components; `PlayerBar` (memoized, no props) owns the ~250ms player-state poll and transport controls, isolated so the poll does not re-render the lists. Albums-by-artist and tracks-by-album are cached; navigation is optimistic.
- `src-tauri/src/subsonic.rs` : OpenSubsonic client (`Subsonic::new(url, user, password)`, token auth; ping, getArtists, getArtist, getAlbum, getRandomSongs, getPlaylists, getPlaylist, getStarred2, search3, and stream / getCoverArt URL builders).
- `src-tauri/src/http_source.rs` : `HttpStream`, a `Read + Seek` source over HTTP range requests, also a Symphonia `MediaSource` (real `byte_len` + `is_seekable`).
- `src-tauri/src/audio.rs` : native audio service. `Engine` owns the play queue with auto-advance / next / previous / replay and session save/restore, on its own thread with a command queue; `SampleQueue` plus a producer thread decode ahead into a bounded ring buffer that `StreamingSource` (a Rodio `Source`) drains without blocking; `souvlaki` for media keys.
- `src-tauri/src/store.rs` : non-secret config (`server.json`), generic blob read/write for the session and playhead files, and the keyring wrapper for the password.
- `src-tauri/src/lib.rs` : Tauri state, error classification, session restore, and the commands (saved_server, connect_server, connect_saved, forget_server, list_artists, list_albums, list_album_tracks, list_playlists, playlist_tracks, list_starred, search, play_album, play_songs, play_random, next_track, prev_track, replay, toggle_play, seek, set_volume, player_state).

Stack in use: Tauri 2, React 19, TypeScript, Vite, Rust. Audio is Symphonia + Rodio + CPAL + souvlaki; credentials via `keyring`; config dir via `dirs`. Stream URLs carry Subsonic token auth and do not expire within the session, so a whole album's URLs are prebuilt in the command layer and handed to the audio thread, which advances the queue without needing credentials. SQLite is not used yet; a JSON config/session store covers current needs.

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

## Audio findings

- Decode with Symphonia directly, not `rodio::Decoder`. Rodio 0.20's `ReadSeekSource` hardcodes `byte_len() = None`, which makes Symphonia report FLAC as `Unseekable`. `HttpStream` wrapped as a Symphonia `MediaSource` returning `byte_len = Some(len)` and `is_seekable = true` seeks correctly. Rodio is used only as the output sink via a custom `Source` (`StreamingSource`) whose `try_seek` drives the Symphonia format reader. Decoding ourselves is also what will later give the PCM tap for the FFT visualizer and the EQ/DSP chain.
- Streaming and decode run on a producer thread that fills a bounded sample ring buffer (`SampleQueue`); `StreamingSource` only drains memory, so the audio output thread never blocks on the network. This fixed the earlier ALSA underruns.
- Navidrome's `stream` endpoint honors range requests (`Accept-Ranges: bytes`), so no temp-file buffering is needed.
- OS media controls (MPRIS/SMTC/Now Playing) are not provided by Rodio/CPAL; `souvlaki` handles them.
- Server is all-FLAC, so Opus (Symphonia's weak spot) is a non-issue for now.

## Known gaps

- Output stream resilience (NFR-05): the `OutputStream` is created once and never recreated, so if the ALSA/CPAL stream dies (`alsa::poll()` POLLERR, e.g. the audio device changes, or the CPU is starved such as by a full `cargo build` during playback) the app goes silent until restart. Recreating the stream on error is not implemented. Note: do not run heavy rebuilds while audio is playing in dev; it can trigger POLLERR via starvation.
- Secret scrubbing in error strings: `classify_error` maps common connect failures to generic messages, but the fallback returns the raw error, which for a reqwest error can include the request URL (with token). Scrub the URL/token from any error surfaced to the UI or logs (ties to the invariant about keeping sensitive URLs out of logs).
- Keyring depends on a running Secret Service (gnome-keyring/KWallet). If absent, saving the password fails gracefully (logged, non-fatal) and the user re-enters it each launch; auto-reconnect then cannot run.

## Conventions

- No comments in code. No emojis anywhere in code or UI strings. No em or en dashes in code or documentation (plain chat is fine).
- Spec language is Spanish; the app must be i18n-ready for Spanish and English (NFR-10).
- Accessibility is P0, not polish: keyboard-navigable, screen-reader friendly, visible focus, honor `prefers-reduced-motion`.
- Design direction ("simple, not minimalist"): muted palette, layered surfaces, a harmonic accent that never replaces labels or hierarchy. Theme tokens (CSS variables like `--bg`, `--surface`, `--accent`) are in `docs/docs/06-diseno.md`, exploratory until WCAG-contrast-checked.
- MVP scope rule: a feature is in only if it directly serves connecting, finding, and listening to music reliably (`docs/docs/07-mvp-roadmap.md`). EQ, visualizers, offline, full stats, and multi-server are deferred.

## Running the app (dev)

- `npm run app` launches the Tauri desktop app (Vite plus Rust). Use this, not `npm run tauri dev`, on Linux: it sets `WEBKIT_DISABLE_DMABUF_RENDERER=1`, without which WebKitGTK crashes on Wayland with `Gdk-Message: Error 71 (protocol error) dispatching to Wayland display`. This is a WebKitGTK/Wayland environment issue, not an app bug.
- First Rust build is slow (Symphonia/CPAL/reqwest); later builds are incremental.
- Credentials are entered in the app's connection wizard and saved to `~/.config/resonance/server.json` (URL + username) plus the OS keychain (password). The old `resonance.dev.json` is no longer used by the app.
- Avoid repeated full rebuilds while the app is playing audio: the dev watcher restarts the binary (killing playback) and the compile can starve the audio thread into a POLLERR.

## Git

Do not run `git commit`, `git push`, or otherwise write to git or GitHub. The user handles all commits himself. The working branch is `master`; PRs target `main`.
