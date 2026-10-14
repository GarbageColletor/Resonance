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
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState<string | null>(null);

    const dragging = useRef(false);
    const [dragPos, setDragPos] = useState(0);

    useEffect(() => {
        const id = setInterval(async () => {
            try {
                const st = await invoke<PlayerState>("player_state");
                setPlayer(st);
            } catch {
            }
        }, 250);
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
        run(async () => setSong(await invoke<SongDto>("play_random")));

    const toggle = () => invoke("toggle_play");

    const duration = player?.duration_secs || song?.duration_secs || 0;
    const position = dragging.current ? dragPos : player?.position_secs || 0;
    const playing = player?.playing ?? false;

    return (
        <main className="app">
            <header>
                <h1>Resonance</h1>
                <span className="tag">spike · Milestone 0</span>
            </header>

            {!conn ? (
                <button className="primary" onClick={connect} disabled={busy}>
                    {busy ? "Conectando…" : "Conectar a Navidrome"}
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
                            disabled={!song}
                            onPointerDown={() => (dragging.current = true)}
                            onChange={(e) => setDragPos(parseFloat(e.target.value))}
                            onPointerUp={(e) => {
                                const secs = parseFloat((e.target as HTMLInputElement).value);
                                dragging.current = false;
                                invoke("seek", {secs});
                            }}
                        />
                        <span className="time">{fmt(duration)}</span>
                    </div>

                    <div className="controls">
                        <button onClick={() => toggle()} disabled={!song}>
                            {playing ? "Pausa" : "Play"}
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
