import {useEffect, useRef, useState} from "react";
import {invoke} from "@tauri-apps/api/core";
import "./App.css";

type ConnectInfo = {
    server: string;
    version: string;
    artist_count: number;
    artists_sample: string[];
};

type SongDto = {
    id: string;
    title: string;
    artist: string | null;
    album: string | null;
    duration_secs: number | null;
};

type PlayerState = {
    playing: boolean;
    ended: boolean;
    position_secs: number;
    duration_secs: number;
    title: string | null;
    artist: string | null;
    album: string | null;
};

function fmt(secs: number): string {
    if (!isFinite(secs) || secs < 0) secs = 0;
    const m = Math.floor(secs / 60);
    const s = Math.floor(secs % 60);
    return `${m}:${s.toString().padStart(2, "0")}`;
}

export default function App() {
    const [conn, setConn] = useState<ConnectInfo | null>(null);
    const [song, setSong] = useState<SongDto | null>(null);
    const [player, setPlayer] = useState<PlayerState | null>(null);
    const [uiPlaying, setUiPlaying] = useState(false);
    const [dragPos, setDragPos] = useState(0);
    const [, forceRender] = useState(0);
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState<string | null>(null);

    const dragging = useRef(false);
    const actionAt = useRef(0);
    const pendingSeek = useRef<number | null>(null);
    const pendingSince = useRef(0);
    const lastTitle = useRef<string | null>(null);

    useEffect(() => {
        const id = setInterval(async () => {
            try {
                const st = await invoke<PlayerState>("player_state");
                setPlayer(st);
                if ((st.title ?? null) !== lastTitle.current) {
                    lastTitle.current = st.title ?? null;
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
        }, 200);
        return () => clearInterval(id);
    }, []);

    async function run<T>(fn: () => Promise<T>) {
        setBusy(true);
        setError(null);
        try {
            return await fn();
        } catch (e) {
            setError(String(e));
            return undefined;
        } finally {
            setBusy(false);
        }
    }

    const connect = () =>
        run(async () => setConn(await invoke<ConnectInfo>("connect")));

    const playRandom = () =>
        run(async () => {
            const s = await invoke<SongDto>("play_random");
            setSong(s);
            dragging.current = false;
            pendingSeek.current = null;
            actionAt.current = Date.now();
            setUiPlaying(true);
        });

    const toggle = () => {
        setUiPlaying((p) => !p);
        actionAt.current = Date.now();
        invoke("toggle_play");
    };

    const replay = () => {
        pendingSeek.current = null;
        actionAt.current = Date.now();
        setUiPlaying(true);
        invoke("replay");
    };

    const ended = player?.ended ?? false;
    const duration = player?.duration_secs || song?.duration_secs || 0;
    const position = dragging.current
        ? dragPos
        : pendingSeek.current !== null
            ? pendingSeek.current
            : player?.position_secs || 0;

    return (
        <main className="app">
            <header>
                <h1>Resonance</h1>
                <span className="tag">spike · Milestone 0</span>
            </header>

            {!conn ? (
                <button className="primary" onClick={connect} disabled={busy}>
                    {busy ? "Conectando..." : "Conectar a Navidrome"}
                </button>
            ) : (
                <p className="server">
                    {conn.server} · Subsonic v{conn.version} · {conn.artist_count} artistas
                </p>
            )}

            {conn && (
                <section className="player">
                    <div className="now">
                        <div className="title">{player?.title ?? song?.title ?? "-"}</div>
                        <div className="sub">
                            {(player?.artist ?? song?.artist) || "-"}
                            {(player?.album ?? song?.album)
                                ? ` · ${player?.album ?? song?.album}`
                                : ""}
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
                            disabled={!song || ended}
                            onPointerDown={() => {
                                dragging.current = true;
                                forceRender((n) => n + 1);
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
                        <button onClick={ended ? replay : toggle} disabled={!song}>
                            {!uiPlaying || ended ? "Play" : "Pausa"}
                        </button>
                        <button className="primary" onClick={playRandom} disabled={busy}>
                            Pista aleatoria
                        </button>
                    </div>
                </section>
            )}

            {error && <p className="error">{error}</p>}
        </main>
    );
}
