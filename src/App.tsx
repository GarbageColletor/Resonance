import {memo, useCallback, useEffect, useRef, useState} from "react";
import {invoke} from "@tauri-apps/api/core";
import "./App.css";

type ConnectInfo = {
    server: string;
    version: string;
    artist_count: number;
};

type SavedServerInfo = {
    url: string;
    username: string;
};

type Artist = {
    id: string;
    name: string;
    album_count: number | null;
    cover_url: string | null;
};

type Album = {
    id: string;
    name: string;
    artist: string | null;
    year: number | null;
    song_count: number | null;
    cover_url: string | null;
};

type Song = {
    id: string;
    title: string;
    artist: string | null;
    album: string | null;
    duration_secs: number | null;
    cover_url: string | null;
};

type Playlist = {
    id: string;
    name: string;
    song_count: number | null;
    cover_url: string | null;
};

type PlayerState = {
    playing: boolean;
    ended: boolean;
    has_next: boolean;
    has_prev: boolean;
    shuffle: boolean;
    repeat: string;
    position_secs: number;
    duration_secs: number;
    current_id: string | null;
    title: string | null;
    artist: string | null;
    album: string | null;
    cover_url: string | null;
};

type SearchResults = {
    artists: Artist[];
    albums: Album[];
    songs: Song[];
};

type Level = "artists" | "albums" | "tracks";

function fmt(secs: number): string {
    if (!isFinite(secs) || secs < 0) secs = 0;
    const m = Math.floor(secs / 60);
    const s = Math.floor(secs % 60);
    return `${m}:${s.toString().padStart(2, "0")}`;
}

function Cover({url, size}: {url: string | null; size: number}) {
    const style = {width: size, height: size};
    return url ? (
        <img className="cover" style={style} src={url} loading="lazy" alt="" />
    ) : (
        <div className="cover cover-empty" style={style} />
    );
}

type BrowseProps = {
    level: Level;
    artists: Artist[];
    albums: Album[];
    tracks: Song[];
    loading: boolean;
    onArtist: (a: Artist) => void;
    onAlbum: (al: Album) => void;
    onTrack: (i: number) => void;
};

const BrowseList = memo(function BrowseList(p: BrowseProps) {
    if (p.loading) {
        return (
            <section className="browse">
                <div className="empty">Cargando...</div>
            </section>
        );
    }
    return (
        <section className="browse">
            {p.level === "artists" &&
                p.artists.map((a) => (
                    <button key={a.id} className="row" onClick={() => p.onArtist(a)}>
                        <Cover url={a.cover_url} size={40} />
                        <span className="row-main">{a.name}</span>
                        <span className="row-sub">{a.album_count ? `${a.album_count} alb.` : ""}</span>
                    </button>
                ))}
            {p.level === "albums" &&
                p.albums.map((al) => (
                    <button key={al.id} className="row" onClick={() => p.onAlbum(al)}>
                        <Cover url={al.cover_url} size={40} />
                        <span className="row-main">{al.name}</span>
                        <span className="row-sub">{al.year ?? ""}</span>
                    </button>
                ))}
            {p.level === "tracks" &&
                p.tracks.map((t, i) => (
                    <button key={t.id} className="row" onClick={() => p.onTrack(i)}>
                        <span className="row-idx">{i + 1}</span>
                        <span className="row-main">{t.title}</span>
                        <span className="row-sub">{fmt(t.duration_secs ?? 0)}</span>
                    </button>
                ))}
        </section>
    );
});

const PlayerBar = memo(function PlayerBar() {
    const [player, setPlayer] = useState<PlayerState | null>(null);
    const [uiPlaying, setUiPlaying] = useState(false);
    const [dragPos, setDragPos] = useState(0);

    const dragging = useRef(false);
    const actionAt = useRef(0);
    const pendingSeek = useRef<number | null>(null);
    const pendingSince = useRef(0);
    const lastId = useRef<string | null>(null);

    useEffect(() => {
        const id = setInterval(async () => {
            try {
                const st = await invoke<PlayerState>("player_state");
                setPlayer(st);
                if ((st.current_id ?? null) !== lastId.current) {
                    lastId.current = st.current_id ?? null;
                    dragging.current = false;
                    pendingSeek.current = null;
                }
                if (Date.now() - actionAt.current > 500) {
                    setUiPlaying(st.playing);
                }
                if (pendingSeek.current !== null) {
                    const reached = Math.abs(st.position_secs - pendingSeek.current) < 1.0;
                    const expired = Date.now() - pendingSince.current > 1500;
                    if (reached || expired) pendingSeek.current = null;
                }
            } catch {
            }
        }, 250);
        return () => clearInterval(id);
    }, []);

    if (!player?.title) return null;

    const ended = player.ended;
    const duration = player.duration_secs || 0;
    const position = dragging.current
        ? dragPos
        : pendingSeek.current !== null
            ? pendingSeek.current
            : player.position_secs || 0;

    const toggle = () => {
        setUiPlaying((v) => !v);
        actionAt.current = Date.now();
        invoke("toggle_play");
    };
    const replay = () => {
        pendingSeek.current = null;
        actionAt.current = Date.now();
        setUiPlaying(true);
        invoke("replay");
    };
    const skip = (dir: "next_track" | "prev_track") => {
        pendingSeek.current = null;
        actionAt.current = Date.now();
        setUiPlaying(true);
        invoke(dir);
    };

    return (
        <section className="player">
            <div className="now">
                <Cover url={player.cover_url} size={52} />
                <div className="now-text">
                    <div className="title">{player.title}</div>
                    <div className="sub">
                        {player.artist || "-"}
                        {player.album ? ` · ${player.album}` : ""}
                    </div>
                </div>
            </div>

            <div className="seekrow">
                <span className="time">{fmt(position)}</span>
                <input
                    type="range"
                    min={0}
                    max={Math.max(duration, 1)}
                    step={0.5}
                    value={Math.min(position, duration || position)}
                    disabled={ended}
                    onPointerDown={() => {
                        dragging.current = true;
                    }}
                    onChange={(e) => setDragPos(parseFloat(e.target.value))}
                    onPointerUp={(e) => {
                        const secs = parseFloat((e.target as HTMLInputElement).value);
                        dragging.current = false;
                        pendingSeek.current = secs;
                        pendingSince.current = Date.now();
                        actionAt.current = Date.now();
                        invoke("seek", {secs});
                    }}
                    onPointerCancel={() => {
                        dragging.current = false;
                    }}
                    onLostPointerCapture={() => {
                        dragging.current = false;
                    }}
                />
                <span className="time">{fmt(duration)}</span>
            </div>

            <div className="controls">
                <button onClick={() => skip("prev_track")} disabled={!player.has_prev}>
                    Anterior
                </button>
                <button className="primary" onClick={ended ? replay : toggle}>
                    {!uiPlaying || ended ? "Play" : "Pausa"}
                </button>
                <button onClick={() => skip("next_track")} disabled={!player.has_next}>
                    Siguiente
                </button>
            </div>

            <div className="modes">
                <button
                    className={player.shuffle ? "mode active" : "mode"}
                    onClick={() => invoke("toggle_shuffle")}
                >
                    Aleatorio
                </button>
                <button
                    className={player.repeat !== "off" ? "mode active" : "mode"}
                    onClick={() => invoke("cycle_repeat")}
                >
                    {player.repeat === "one"
                        ? "Repetir: una"
                        : player.repeat === "all"
                            ? "Repetir: todo"
                            : "Repetir"}
                </button>
            </div>
        </section>
    );
});

type SearchProps = {
    results: SearchResults;
    onArtist: (a: Artist) => void;
    onAlbum: (al: Album) => void;
    onSong: (s: Song) => void;
};

function SearchResultsView(p: SearchProps) {
    const empty =
        p.results.artists.length === 0 &&
        p.results.albums.length === 0 &&
        p.results.songs.length === 0;
    if (empty) {
        return (
            <section className="browse">
                <div className="empty">Sin resultados</div>
            </section>
        );
    }
    return (
        <section className="browse">
            {p.results.artists.length > 0 && <div className="group">Artistas</div>}
            {p.results.artists.map((a) => (
                <button key={a.id} className="row" onClick={() => p.onArtist(a)}>
                    <Cover url={a.cover_url} size={40} />
                    <span className="row-main">{a.name}</span>
                </button>
            ))}
            {p.results.albums.length > 0 && <div className="group">Albumes</div>}
            {p.results.albums.map((al) => (
                <button key={al.id} className="row" onClick={() => p.onAlbum(al)}>
                    <Cover url={al.cover_url} size={40} />
                    <span className="row-main">{al.name}</span>
                    <span className="row-sub">{al.artist ?? ""}</span>
                </button>
            ))}
            {p.results.songs.length > 0 && <div className="group">Canciones</div>}
            {p.results.songs.map((s) => (
                <button key={s.id} className="row" onClick={() => p.onSong(s)}>
                    <Cover url={s.cover_url} size={40} />
                    <span className="row-main">{s.title}</span>
                    <span className="row-sub">{s.artist ?? ""}</span>
                </button>
            ))}
        </section>
    );
}

function SongList({songs, onPlay}: {songs: Song[]; onPlay: (i: number) => void}) {
    if (songs.length === 0) {
        return (
            <section className="browse">
                <div className="empty">Vacio</div>
            </section>
        );
    }
    return (
        <section className="browse">
            {songs.map((s, i) => (
                <button key={s.id + i} className="row" onClick={() => onPlay(i)}>
                    <Cover url={s.cover_url} size={40} />
                    <span className="row-main">{s.title}</span>
                    <span className="row-sub">{s.artist ?? ""}</span>
                    <span className="row-sub">{fmt(s.duration_secs ?? 0)}</span>
                </button>
            ))}
        </section>
    );
}

type Tab = "artists" | "playlists" | "favorites";

export default function App() {
    const [conn, setConn] = useState<ConnectInfo | null>(null);
    const [tab, setTab] = useState<Tab>("artists");
    const [playlists, setPlaylists] = useState<Playlist[]>([]);
    const [currentPlaylist, setCurrentPlaylist] = useState<Playlist | null>(null);
    const [playlistTracks, setPlaylistTracks] = useState<Song[]>([]);
    const [starred, setStarred] = useState<Song[]>([]);
    const [level, setLevel] = useState<Level>("artists");
    const [artists, setArtists] = useState<Artist[]>([]);
    const [albums, setAlbums] = useState<Album[]>([]);
    const [tracks, setTracks] = useState<Song[]>([]);
    const [currentArtist, setCurrentArtist] = useState<Artist | null>(null);
    const [currentAlbum, setCurrentAlbum] = useState<Album | null>(null);
    const [loading, setLoading] = useState(false);
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [query, setQuery] = useState("");
    const [results, setResults] = useState<SearchResults | null>(null);
    const [serverUrl, setServerUrl] = useState("");
    const [username, setUsername] = useState("");
    const [password, setPassword] = useState("");
    const [remember, setRemember] = useState(true);
    const [checkingSaved, setCheckingSaved] = useState(true);

    const albumCache = useRef(new Map<string, Album[]>());
    const trackCache = useRef(new Map<string, Song[]>());

    useEffect(() => {
        const q = query.trim();
        if (!q) {
            setResults(null);
            return;
        }
        const t = setTimeout(() => {
            invoke<SearchResults>("search", {query: q})
                .then(setResults)
                .catch((e) => setError(String(e)));
        }, 300);
        return () => clearTimeout(t);
    }, [query]);

    const afterConnect = async () => {
        setArtists(await invoke<Artist[]>("list_artists"));
        setLevel("artists");
        setTab("artists");
    };

    useEffect(() => {
        (async () => {
            try {
                const saved = await invoke<SavedServerInfo | null>("saved_server");
                if (saved) {
                    setServerUrl(saved.url);
                    setUsername(saved.username);
                    try {
                        setConn(await invoke<ConnectInfo>("connect_saved"));
                        await afterConnect();
                    } catch (e) {
                        setError(String(e));
                    }
                }
            } catch {
            } finally {
                setCheckingSaved(false);
            }
        })();
    }, []);

    const doConnect = async () => {
        setBusy(true);
        setError(null);
        try {
            setConn(
                await invoke<ConnectInfo>("connect_server", {
                    url: serverUrl,
                    username,
                    password,
                    remember,
                }),
            );
            await afterConnect();
        } catch (e) {
            setError(String(e));
        } finally {
            setBusy(false);
        }
    };

    const forget = async () => {
        await invoke("forget_server").catch(() => {});
        setConn(null);
        setPassword("");
        setArtists([]);
        setAlbums([]);
        setTracks([]);
        setCurrentArtist(null);
        setCurrentAlbum(null);
    };

    const onArtist = useCallback((a: Artist) => {
        setCurrentArtist(a);
        setLevel("albums");
        const cached = albumCache.current.get(a.id);
        if (cached) {
            setAlbums(cached);
            return;
        }
        setAlbums([]);
        setLoading(true);
        setError(null);
        invoke<Album[]>("list_albums", {artistId: a.id})
            .then((al) => {
                albumCache.current.set(a.id, al);
                setAlbums(al);
            })
            .catch((e) => setError(String(e)))
            .finally(() => setLoading(false));
    }, []);

    const onAlbum = useCallback((al: Album) => {
        setCurrentAlbum(al);
        setLevel("tracks");
        const cached = trackCache.current.get(al.id);
        if (cached) {
            setTracks(cached);
            return;
        }
        setTracks([]);
        setLoading(true);
        setError(null);
        invoke<Song[]>("list_album_tracks", {albumId: al.id})
            .then((ts) => {
                trackCache.current.set(al.id, ts);
                setTracks(ts);
            })
            .catch((e) => setError(String(e)))
            .finally(() => setLoading(false));
    }, []);

    const onTrack = useCallback(
        (i: number) => {
            if (!currentAlbum) return;
            invoke("play_album", {albumId: currentAlbum.id, start: i});
        },
        [currentAlbum],
    );

    const playRandom = () => invoke("play_random").catch((e) => setError(String(e)));

    const goTab = (t: Tab) => {
        setTab(t);
        setError(null);
        if (t === "playlists" && playlists.length === 0) {
            setLoading(true);
            invoke<Playlist[]>("list_playlists")
                .then(setPlaylists)
                .catch((e) => setError(String(e)))
                .finally(() => setLoading(false));
        }
        if (t === "favorites") {
            setLoading(true);
            invoke<Song[]>("list_starred")
                .then(setStarred)
                .catch((e) => setError(String(e)))
                .finally(() => setLoading(false));
        }
    };

    const openPlaylist = (pl: Playlist) => {
        setCurrentPlaylist(pl);
        setPlaylistTracks([]);
        setLoading(true);
        setError(null);
        invoke<Song[]>("playlist_tracks", {playlistId: pl.id})
            .then(setPlaylistTracks)
            .catch((e) => setError(String(e)))
            .finally(() => setLoading(false));
    };

    return (
        <main className="app">
            <header>
                <h1>Resonance</h1>
                {conn && (
                    <>
                        <span className="server">
                            {conn.server} · v{conn.version} · {conn.artist_count} artistas
                        </span>
                        <button className="link" onClick={forget}>
                            Cambiar servidor
                        </button>
                    </>
                )}
            </header>

            {!conn ? (
                checkingSaved ? (
                    <p className="server">Conectando...</p>
                ) : (
                    <form
                        className="connect"
                        onSubmit={(e) => {
                            e.preventDefault();
                            doConnect();
                        }}
                    >
                        <input
                            type="text"
                            placeholder="https://tu-servidor"
                            value={serverUrl}
                            onChange={(e) => setServerUrl(e.target.value)}
                        />
                        <input
                            type="text"
                            placeholder="Usuario"
                            autoComplete="username"
                            value={username}
                            onChange={(e) => setUsername(e.target.value)}
                        />
                        <input
                            type="password"
                            placeholder="Contrasena"
                            autoComplete="current-password"
                            value={password}
                            onChange={(e) => setPassword(e.target.value)}
                        />
                        <label className="remember">
                            <input
                                type="checkbox"
                                checked={remember}
                                onChange={(e) => setRemember(e.target.checked)}
                            />
                            Recordar en este equipo
                        </label>
                        <button
                            className="primary"
                            type="submit"
                            disabled={busy || !serverUrl || !username}
                        >
                            {busy ? "Conectando..." : "Conectar"}
                        </button>
                    </form>
                )
            ) : (
                <>
                    <nav className="tabs">
                        <button
                            className={tab === "artists" ? "tab active" : "tab"}
                            onClick={() => setTab("artists")}
                        >
                            Artistas
                        </button>
                        <button
                            className={tab === "playlists" ? "tab active" : "tab"}
                            onClick={() => goTab("playlists")}
                        >
                            Playlists
                        </button>
                        <button
                            className={tab === "favorites" ? "tab active" : "tab"}
                            onClick={() => goTab("favorites")}
                        >
                            Favoritos
                        </button>
                    </nav>

                    {tab === "artists" && (
                        <>
                            <input
                                className="search"
                                type="text"
                                placeholder="Buscar artistas, albumes, canciones..."
                                value={query}
                                onChange={(e) => setQuery(e.target.value)}
                            />
                            {results ? (
                                <SearchResultsView
                                    results={results}
                                    onArtist={(a) => {
                                        setQuery("");
                                        onArtist(a);
                                    }}
                                    onAlbum={(al) => {
                                        setQuery("");
                                        onAlbum(al);
                                    }}
                                    onSong={(s) => {
                                        invoke("play_songs", {songs: [s], start: 0});
                                    }}
                                />
                            ) : (
                                <>
                                    <nav className="crumbs">
                                        <button className="link" onClick={() => setLevel("artists")}>
                                            Artistas
                                        </button>
                                        {currentArtist && (level === "albums" || level === "tracks") && (
                                            <>
                                                <span className="sep">/</span>
                                                <button className="link" onClick={() => setLevel("albums")}>
                                                    {currentArtist.name}
                                                </button>
                                            </>
                                        )}
                                        {currentAlbum && level === "tracks" && (
                                            <>
                                                <span className="sep">/</span>
                                                <span className="crumb-current">{currentAlbum.name}</span>
                                            </>
                                        )}
                                        <button className="link rand" onClick={playRandom}>
                                            Aleatoria
                                        </button>
                                    </nav>

                                    <BrowseList
                                        level={level}
                                        artists={artists}
                                        albums={albums}
                                        tracks={tracks}
                                        loading={loading}
                                        onArtist={onArtist}
                                        onAlbum={onAlbum}
                                        onTrack={onTrack}
                                    />
                                </>
                            )}
                        </>
                    )}

                    {tab === "playlists" &&
                        (currentPlaylist ? (
                            <>
                                <nav className="crumbs">
                                    <button className="link" onClick={() => setCurrentPlaylist(null)}>
                                        Playlists
                                    </button>
                                    <span className="sep">/</span>
                                    <span className="crumb-current">{currentPlaylist.name}</span>
                                </nav>
                                {loading ? (
                                    <section className="browse">
                                        <div className="empty">Cargando...</div>
                                    </section>
                                ) : (
                                    <SongList
                                        songs={playlistTracks}
                                        onPlay={(i) =>
                                            invoke("play_songs", {songs: playlistTracks, start: i})
                                        }
                                    />
                                )}
                            </>
                        ) : loading ? (
                            <section className="browse">
                                <div className="empty">Cargando...</div>
                            </section>
                        ) : (
                            <section className="browse">
                                {playlists.length === 0 && (
                                    <div className="empty">Sin playlists</div>
                                )}
                                {playlists.map((pl) => (
                                    <button
                                        key={pl.id}
                                        className="row"
                                        onClick={() => openPlaylist(pl)}
                                    >
                                        <Cover url={pl.cover_url} size={40} />
                                        <span className="row-main">{pl.name}</span>
                                        <span className="row-sub">
                                            {pl.song_count ? `${pl.song_count}` : ""}
                                        </span>
                                    </button>
                                ))}
                            </section>
                        ))}

                    {tab === "favorites" &&
                        (loading ? (
                            <section className="browse">
                                <div className="empty">Cargando...</div>
                            </section>
                        ) : (
                            <SongList
                                songs={starred}
                                onPlay={(i) => invoke("play_songs", {songs: starred, start: i})}
                            />
                        ))}
                </>
            )}

            <PlayerBar />

            {error && <p className="error">{error}</p>}
        </main>
    );
}
