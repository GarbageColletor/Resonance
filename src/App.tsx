import {memo, useCallback, useEffect, useRef, useState} from "react";
import {invoke} from "@tauri-apps/api/core";
import "./App.css";

type ConnectInfo = {
    server: string;
    version: string;
    artist_count: number;
};

type Artist = {
    id: string;
    name: string;
    album_count: number | null;
};

type Album = {
    id: string;
    name: string;
    artist: string | null;
    year: number | null;
    song_count: number | null;
};

type Song = {
    id: string;
    title: string;
    artist: string | null;
    album: string | null;
    duration_secs: number | null;
};

type PlayerState = {
    playing: boolean;
    ended: boolean;
    has_next: boolean;
    has_prev: boolean;
    position_secs: number;
    duration_secs: number;
    current_id: string | null;
    title: string | null;
    artist: string | null;
    album: string | null;
};

type Level = "artists" | "albums" | "tracks";

function fmt(secs: number): string {
    if (!isFinite(secs) || secs < 0) secs = 0;
    const m = Math.floor(secs / 60);
    const s = Math.floor(secs % 60);
    return `${m}:${s.toString().padStart(2, "0")}`;
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
                        <span className="row-main">{a.name}</span>
                        <span className="row-sub">{a.album_count ? `${a.album_count} alb.` : ""}</span>
                    </button>
                ))}
            {p.level === "albums" &&
                p.albums.map((al) => (
                    <button key={al.id} className="row" onClick={() => p.onAlbum(al)}>
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
                <div className="title">{player.title}</div>
                <div className="sub">
                    {player.artist || "-"}
                    {player.album ? ` · ${player.album}` : ""}
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
        </section>
    );
});

export default function App() {
    const [conn, setConn] = useState<ConnectInfo | null>(null);
    const [level, setLevel] = useState<Level>("artists");
    const [artists, setArtists] = useState<Artist[]>([]);
    const [albums, setAlbums] = useState<Album[]>([]);
    const [tracks, setTracks] = useState<Song[]>([]);
    const [currentArtist, setCurrentArtist] = useState<Artist | null>(null);
    const [currentAlbum, setCurrentAlbum] = useState<Album | null>(null);
    const [loading, setLoading] = useState(false);
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState<string | null>(null);

    const albumCache = useRef(new Map<string, Album[]>());
    const trackCache = useRef(new Map<string, Song[]>());

    const connect = () =>
        (async () => {
            setBusy(true);
            setError(null);
            try {
                setConn(await invoke<ConnectInfo>("connect"));
                setArtists(await invoke<Artist[]>("list_artists"));
                setLevel("artists");
            } catch (e) {
                setError(String(e));
            } finally {
                setBusy(false);
            }
        })();

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

    return (
        <main className="app">
            <header>
                <h1>Resonance</h1>
                {conn && (
                    <span className="server">
                        {conn.server} · v{conn.version} · {conn.artist_count} artistas
                    </span>
                )}
            </header>

            {!conn ? (
                <button className="primary" onClick={connect} disabled={busy}>
                    {busy ? "Conectando..." : "Conectar a Navidrome"}
                </button>
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

            <PlayerBar />

            {error && <p className="error">{error}</p>}
        </main>
    );
}
