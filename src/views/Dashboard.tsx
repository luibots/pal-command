import { useEffect, useMemo, useRef, useState } from "react";
import { api, humanUptime } from "../lib/api";
import type { AppSettings, SecretsPresent, LiveInfo, LiveMetrics, LivePlayer } from "../lib/api";

interface Props {
  info: LiveInfo | null;
  metrics: LiveMetrics | null;
  liveOk: boolean | null;
  settings: AppSettings;
  secrets: SecretsPresent | null;
}

type EvKind = "join" | "leave" | "level" | "op" | "info";
type Event = { at: number; kind: EvKind; msg: string };

function worldToMap(x: number, y: number): [number, number] {
  return [
    (y - 158_000) / 460,
    (x + 123_000) / 460,
  ];
}

const BROADCAST_PRESETS = [
  { label: "Restart 5m", msg: "Server restart in 5 minutes — safe spot up!" },
  { label: "Restart 1m", msg: "Server restart in 60 seconds!" },
  { label: "Backing up", msg: "Backing up the world — you might see a brief hitch." },
  { label: "AFK check", msg: "AFK sweep in 5 minutes — move or get kicked." },
  { label: "GG", msg: "GG everyone — see you tomorrow!" },
];

export function Dashboard({ info, metrics, liveOk, settings, secrets }: Props) {
  const [players, setPlayers] = useState<LivePlayer[]>([]);
  const [announce, setAnnounce] = useState("");
  const [events, setEvents] = useState<Event[]>([]);
  const [fpsHistory, setFpsHistory] = useState<number[]>([]);
  const [nextBackup, setNextBackup] = useState<number | null>(null);
  const [lastBackup, setLastBackup] = useState<string | null>(null);
  const [snapshotCount, setSnapshotCount] = useState(0);
  const [busy, setBusy] = useState(false);
  const [copiedPlayer, setCopiedPlayer] = useState<string | null>(null);
  const prevIdsRef = useRef<Set<string>>(new Set());
  const prevLevelsRef = useRef<Map<string, number>>(new Map());

  const pushEvent = (kind: EvKind, msg: string) =>
    setEvents(l => [{ at: Date.now(), kind, msg }, ...l].slice(0, 100));

  // FPS sparkline history (REST only — RCON doesn't expose serverfps).
  useEffect(() => {
    if (metrics?.serverfps != null) {
      setFpsHistory(h => [...h.slice(-29), metrics.serverfps!]);
    }
  }, [metrics?.serverfps]);

  // Player polling → derive join/leave/level events locally
  useEffect(() => {
    if (liveOk !== true) {
      setPlayers([]);
      prevIdsRef.current = new Set();
      prevLevelsRef.current = new Map();
      return;
    }
    let alive = true;
    const poll = async () => {
      try {
        const p = await api.livePlayers();
        if (!alive) return;
        setPlayers(p);

        const newIds = new Set(p.map(x => x.userId ?? x.playerId ?? x.name ?? "?").filter(Boolean) as string[]);
        const prevIds = prevIdsRef.current;
        const isFirstPoll = prevIds.size === 0 && p.length === 0;
        if (!isFirstPoll && prevIds.size > 0) {
          for (const pl of p) {
            const id = pl.userId ?? pl.playerId ?? pl.name ?? "?";
            if (!prevIds.has(id)) pushEvent("join", `**${pl.name ?? "player"}** joined`);
            const prevLv = prevLevelsRef.current.get(id);
            if (prevLv != null && pl.level != null && pl.level > prevLv) {
              pushEvent("level", `**${pl.name}** hit level ${pl.level}`);
            }
          }
          for (const oldId of prevIds) {
            if (!newIds.has(oldId)) pushEvent("leave", `player left`);
          }
        }
        prevIdsRef.current = newIds;
        prevLevelsRef.current = new Map(
          p.map(pl => [pl.userId ?? pl.playerId ?? pl.name ?? "?", pl.level ?? 0])
        );
      } catch { /* top bar surfaces live health */ }
    };
    poll();
    const id = setInterval(poll, 12_000);
    return () => { alive = false; clearInterval(id); };
  }, [liveOk]);

  // Next-backup countdown
  useEffect(() => {
    let alive = true;
    const refresh = async () => {
      try {
        const h = await api.backupHistory();
        if (!alive) return;
        setSnapshotCount(h.length);
        setLastBackup(h[0]?.modified ?? null);
        if (settings.schedule_enabled && settings.schedule_minutes > 0) {
          const gap = settings.schedule_minutes * 60_000;
          const last = h[0]?.modified ? new Date(h[0].modified).getTime() : Date.now() - gap;
          setNextBackup(last + gap);
        } else {
          setNextBackup(null);
        }
      } catch { /* silent */ }
    };
    refresh();
    const id = setInterval(refresh, 30_000);
    return () => { alive = false; clearInterval(id); };
  }, [settings.schedule_enabled, settings.schedule_minutes]);

  // Countdown tick
  const [, tick] = useState(0);
  useEffect(() => {
    const id = setInterval(() => tick(v => v + 1), 1000);
    return () => clearInterval(id);
  }, []);

  const action = async (label: string, fn: () => Promise<unknown>) => {
    setBusy(true);
    pushEvent("op", `${label}…`);
    try {
      await fn();
      pushEvent("op", `${label} ✓`);
    } catch (e) {
      pushEvent("op", `${label} failed — ${e}`);
    } finally { setBusy(false); }
  };

  const runBackup = async () => {
    setBusy(true);
    pushEvent("op", "backup starting…");
    try {
      const r = await api.backupNow();
      pushEvent("op", `${r.message} ${r.pushed ? "· pushed" : r.committed ? "· committed" : ""}`);
      const h = await api.backupHistory();
      setSnapshotCount(h.length);
      setLastBackup(h[0]?.modified ?? null);
    } catch (e) {
      pushEvent("op", `backup failed — ${e}`);
    } finally { setBusy(false); }
  };

  const runSafeRestart = async () => {
    if (!confirm(
      "Run the guarded restart?\n\nPAL COMMAND will refuse if players are online, " +
      "create and verify an off-site backup, check players again, then restart."
    )) return;
    setBusy(true);
    pushEvent("op", "safe restart: checking players...");
    try {
      const r = await api.safeRestart();
      pushEvent(
        "op",
        `backup ${r.backup.timestamp} verified${r.backup.pushed ? " + pushed" : ""}; ` +
        `server recovered in ${r.recovery_seconds}s`
      );
      const h = await api.backupHistory();
      setSnapshotCount(h.length);
      setLastBackup(h[0]?.modified ?? null);
    } catch (e) {
      pushEvent("op", `safe restart blocked - ${e}`);
    } finally { setBusy(false); }
  };

  const copyCoordinates = async (playerKey: string, x: number, y: number) => {
    const [mapX, mapY] = worldToMap(x, y);
    await navigator.clipboard.writeText(`${Math.round(mapX)}, ${Math.round(mapY)}`);
    setCopiedPlayer(playerKey);
    window.setTimeout(() => setCopiedPlayer(current => current === playerKey ? null : current), 1500);
  };

  const liveReady = liveOk === true;
  const liveConfigured =
    !!secrets?.admin &&
    ((settings.rest_enabled && !!settings.rest_url) ||
      (settings.rcon_enabled && !!settings.rcon_host));

  const heroClass =
    liveOk === true ? "hero"
    : liveOk === false ? "hero hero--offline"
    : "hero hero--dim";
  const statusText =
    liveOk === true ? "ONLINE"
    : liveOk === false ? "OFFLINE"
    : liveConfigured ? "CONNECTING…" : "LIVE OFF";

  const avgFps = useMemo(() =>
    fpsHistory.length ? fpsHistory.reduce((a, b) => a + b, 0) / fpsHistory.length : null,
  [fpsHistory]);
  const fpsClass =
    metrics?.serverfps == null ? "" : metrics.serverfps >= 45 ? "hero-spark-value--good"
    : metrics.serverfps >= 25 ? "" : "hero-spark-value--warn";

  const usingRcon = info?.source === "rcon";

  return (
    <div className="view">
      {/* ── HERO ─────────────────────────────────────── */}
      <div className={heroClass}>
        <div className="hero-status">
          <div className="hero-status-tag">Server</div>
          <div className="hero-status-value">{statusText}</div>
          <span className="hero-status-pill">
            <span className={`dot ${
              liveOk === true ? "dot--green dot--pulse"
              : liveOk === false ? "dot--red"
              : "dot--dim"
            }`} />
            {info?.source ? `via ${info.source.toUpperCase()}` : (liveConfigured ? "…" : "no channel")}
          </span>
        </div>

        <div className="hero-name">
          <div className="hero-title">{info?.servername ?? "Waiting for telemetry…"}</div>
          <div className="hero-meta">
            <span className="hero-meta-item">UPTIME <strong>{humanUptime(metrics?.uptime)}</strong></span>
            {metrics?.days != null && (
              <span className="hero-meta-item">DAY <strong>{metrics.days}</strong></span>
            )}
            <span className="hero-meta-item">BASES <strong>{metrics?.basecampnum ?? "—"}</strong></span>
            {info?.worldguid && (
              <span className="hero-meta-item">WORLD <strong>{info.worldguid.slice(0, 8)}</strong></span>
            )}
            {info?.version && (
              <span className="hero-meta-item">VER <strong>{info.version}</strong></span>
            )}
          </div>
          {!usingRcon ? (
            <div className="hero-spark">
              <span className="hero-spark-label">FPS</span>
              <span className={`hero-spark-value ${fpsClass}`}>
                {metrics?.serverfps != null ? metrics.serverfps.toFixed(1) : "—"}
              </span>
              <Sparkline data={fpsHistory} />
              {avgFps != null && (
                <span style={{ fontFamily: "var(--font-mono)", fontSize: 10, color: "var(--dim)" }}>
                  avg {avgFps.toFixed(1)}
                </span>
              )}
              {metrics?.serverframetime != null && (
                <span style={{ fontFamily: "var(--font-mono)", fontSize: 10, color: "var(--dim)" }}>
                  · {metrics.serverframetime.toFixed(2)} ms/frame
                </span>
              )}
            </div>
          ) : (
            <div className="hero-spark">
              <span className="hero-spark-label">RCON</span>
              <span style={{ fontFamily: "var(--font-mono)", fontSize: 11, color: "var(--dim)" }}>
                FPS + uptime need REST — Host Havoc would have to open port 8212 for that.
              </span>
            </div>
          )}
        </div>

        <div className="hero-players">
          <div className="hero-players-count">
            {metrics?.currentplayernum ?? players.length ?? 0}
          </div>
          <div className="hero-players-max">
            / {metrics?.maxplayernum ?? "?"} MAX
          </div>
        </div>
      </div>

      {!liveConfigured && (
        <div className="notice">
          <div>
            <strong>Live control not connected.</strong> You have RCON port 30105 allocated by Host Havoc.
            Use the <strong>CONFIG</strong> tab to add <code>RCONEnabled=True</code>,{" "}
            <code>RCONPort=30105</code>, and a strong <code>AdminPassword</code> to your Palworld
            settings — then enable RCON in Settings with that AdminPassword.
          </div>
        </div>
      )}

      {/* ── QUICK OPS ────────────────────────────────── */}
      <div className="qops">
        <div className="panel">
          <div className="panel-head">
            <span className="panel-title">Quick Ops</span>
            {info?.source && <span className="chip chip--amber">{info.source.toUpperCase()}</span>}
          </div>
          <div className="panel-body">
            <div className="qops-actions">
              <button className="qbtn" disabled={!liveReady || busy}
                onClick={() => action("force save", () => api.liveSave())}>
                <span className="qbtn-label">💾 Save Now</span>
                <span className="qbtn-hint">Force a clean save</span>
              </button>
              <button className="qbtn" disabled={busy}
                onClick={runBackup}>
                <span className="qbtn-label">📦 Backup Now</span>
                <span className="qbtn-hint">
                  {lastBackup ? `last · ${lastBackup.slice(11, 16)}` : "no snapshots yet"}
                </span>
              </button>
              <button className="qbtn" disabled={!liveReady || busy}
                onClick={runSafeRestart}>
                <span className="qbtn-label">Safe Restart</span>
                <span className="qbtn-hint">0 players - backup - verify - restart</span>
              </button>
              <button className="qbtn qbtn--danger" disabled={!liveReady || busy}
                onClick={() => {
                  if (!confirm("Immediately stop the server?\nAny unsaved progress since the last save is lost."))
                    return;
                  action("force stop", () => api.liveStop());
                }}>
                <span className="qbtn-label">⛔ Force Stop</span>
                <span className="qbtn-hint">Emergency kill</span>
              </button>
            </div>

            <div className="broadcast-row" style={{ marginTop: 14 }}>
              <input
                className="input"
                placeholder={usingRcon
                  ? "Broadcast (RCON turns spaces into underscores — server bug)"
                  : "Broadcast to every player…"}
                value={announce}
                onChange={e => setAnnounce(e.target.value)}
                disabled={!liveReady}
                onKeyDown={e => {
                  if (e.key === "Enter" && announce.trim()) {
                    const msg = announce; setAnnounce("");
                    action(`announce "${truncate(msg, 40)}"`, () => api.liveAnnounce(msg));
                  }
                }}
              />
              <button
                className="btn btn--primary"
                disabled={!liveReady || !announce.trim() || busy}
                onClick={() => {
                  const msg = announce; setAnnounce("");
                  action(`announce "${truncate(msg, 40)}"`, () => api.liveAnnounce(msg));
                }}
              >Send</button>
            </div>
            <div className="broadcast-presets">
              {BROADCAST_PRESETS.map(p => (
                <button key={p.label} className="preset"
                  disabled={!liveReady || busy}
                  onClick={() => action(`announce "${p.label}"`, () => api.liveAnnounce(p.msg))}
                  title={p.msg}
                >{p.label}</button>
              ))}
            </div>
          </div>
        </div>

        <div className="panel">
          <div className="panel-head"><span className="panel-title">Backup Status</span></div>
          <div className="panel-body">
            <div className="countdown" style={{ marginBottom: 12 }}>
              <span className="countdown-value">
                {settings.schedule_enabled && nextBackup ? formatCountdown(nextBackup) : "MANUAL"}
              </span>
              <span className="countdown-label">
                {settings.schedule_enabled ? "till next backup" : "no schedule set"}
              </span>
            </div>
            <div className="hero-meta" style={{ marginTop: 0 }}>
              <span className="hero-meta-item">SNAPSHOTS <strong>{snapshotCount}</strong></span>
              <span className="hero-meta-item">RETAIN <strong>{settings.backup_retention}</strong></span>
              <span className="hero-meta-item">
                MODE <strong>{settings.stop_before_backup ? "STOP-FIRST" : "SAVE+PULL"}</strong>
              </span>
            </div>
            <div style={{ marginTop: 12, fontSize: 11, color: "var(--dim)", fontFamily: "var(--font-mono)" }}>
              LAST · {lastBackup ?? "never"}
            </div>
            {!settings.repo_local_path && (
              <div className="field-hint" style={{ marginTop: 10, color: "var(--red)" }}>
                Choose a local repo folder in Settings to enable backups.
              </div>
            )}
          </div>
        </div>
      </div>

      {/* ── PLAYERS + FEED ───────────────────────────── */}
      <div className="split-3">
        <div className="panel">
          <div className="panel-head">
            <span className="panel-title">Players Online</span>
            <span className="chip chip--amber">{players.length}</span>
          </div>
          <div className="panel-body">
            {!liveReady ? (
              <div className="empty">Enable RCON or REST to see players.</div>
            ) : players.length === 0 ? (
              <div className="empty">Nobody online. Server's peaceful.</div>
            ) : (
              <div className="player-grid">
                {players.map((p, i) => {
                  const pingLevel = p.ping == null ? 0
                    : p.ping < 60 ? 4 : p.ping < 100 ? 3 : p.ping < 180 ? 2 : 1;
                  const playerKey = p.userId ?? p.playerId ?? p.name ?? String(i);
                  const mapPosition = p.location_x != null && p.location_y != null
                    ? worldToMap(p.location_x, p.location_y)
                    : null;
                  return (
                    <div className="player-card" key={playerKey}>
                      <div className="player-head">
                        <div className="player-name" title={p.name ?? p.accountName}>
                          {p.name ?? p.accountName ?? "?"}
                        </div>
                        {p.level != null && (
                          <span className="player-level">LV {p.level}</span>
                        )}
                      </div>
                      {p.ping != null && (
                        <div className="player-row">
                          <span>PING</span>
                          <span style={{ display: "flex", alignItems: "center", gap: 6 }}>
                            <span className={`ping-bar p${pingLevel}`}>
                              <span /><span /><span /><span />
                            </span>
                            <strong>{`${p.ping}ms`}</strong>
                          </span>
                        </div>
                      )}
                      {p.building_count != null && (
                        <div className="player-row">
                          <span>BUILDINGS</span>
                          <strong>{p.building_count}</strong>
                        </div>
                      )}
                      {mapPosition && (
                        <div className="player-row">
                          <span>MAP POS</span>
                          <span className="player-position">
                            <strong title={`Raw world position: ${p.location_x!.toFixed(0)}, ${p.location_y!.toFixed(0)}`}>
                              {`${Math.round(mapPosition[0])}, ${Math.round(mapPosition[1])}`}
                            </strong>
                            <button
                              className="copy-position"
                              title="Copy map coordinates"
                              aria-label={`Copy ${p.name ?? "player"} map coordinates`}
                              onClick={() => copyCoordinates(playerKey, p.location_x!, p.location_y!)}
                            >
                              {copiedPlayer === playerKey ? "Copied" : "Copy"}
                            </button>
                          </span>
                        </div>
                      )}
                      {p.userId && (
                        <div className="player-row">
                          <span>STEAM</span>
                          <strong style={{ fontSize: 10 }}>{p.userId.slice(-8)}</strong>
                        </div>
                      )}
                      <div className="player-actions">
                        <button
                          className="btn btn--sm btn--ghost"
                          disabled={busy || !p.userId}
                          onClick={() => p.userId &&
                            action(`kick ${p.name}`, () =>
                              api.liveKick(p.userId!, "Kicked by admin"))}
                        >Kick</button>
                        <button
                          className="btn btn--sm btn--ghost"
                          style={{ color: "var(--red)" }}
                          disabled={busy || !p.userId}
                          onClick={() => {
                            if (!confirm(`Ban ${p.name}? This is permanent.`)) return;
                            p.userId && action(`ban ${p.name}`, () =>
                              api.liveBan(p.userId!, "Banned by admin"));
                          }}
                        >Ban</button>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </div>

        <div className="panel">
          <div className="panel-head">
            <span className="panel-title">Live Feed</span>
            <span className="chip">{events.length}</span>
          </div>
          <div className="feed">
            {events.length === 0 ? (
              <div className="empty">Feed goes here — joins, level-ups, ops.</div>
            ) : events.map((e, i) => (
              <div key={i} className="event">
                <span className="event-time">{fmtTime(e.at)}</span>
                <span className={`event-icon event-icon--${e.kind}`}>{iconFor(e.kind)}</span>
                <span className="event-msg" dangerouslySetInnerHTML={{ __html: renderMsg(e.msg) }} />
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}

function Sparkline({ data }: { data: number[] }) {
  if (data.length < 2) return <svg className="spark-svg" viewBox="0 0 120 26" />;
  const max = Math.max(60, ...data);
  const min = Math.min(...data);
  const range = Math.max(1, max - min);
  const step = 120 / (data.length - 1);
  const pts = data.map((v, i) => `${(i * step).toFixed(1)},${(24 - ((v - min) / range) * 22).toFixed(1)}`);
  const line = `M ${pts.join(" L ")}`;
  const fill = `M 0,26 L ${pts.join(" L ")} L 120,26 Z`;
  return (
    <svg className="spark-svg" viewBox="0 0 120 26" preserveAspectRatio="none">
      <path className="spark-fill" d={fill} />
      <path className="spark-line" d={line} />
    </svg>
  );
}

function fmtTime(ts: number) {
  const d = new Date(ts);
  return d.toTimeString().slice(0, 5);
}
function iconFor(k: EvKind) {
  return k === "join" ? "→"
    : k === "leave" ? "←"
    : k === "level" ? "↑"
    : k === "op" ? "▸"
    : "•";
}
function renderMsg(m: string): string {
  return m.replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>");
}
function truncate(s: string, n: number) { return s.length <= n ? s : s.slice(0, n - 1) + "…"; }

function formatCountdown(target: number): string {
  const s = Math.max(0, Math.floor((target - Date.now()) / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${String(sec).padStart(2, "0")}s`;
  return `${sec}s`;
}
