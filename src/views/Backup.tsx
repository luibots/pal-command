import { useEffect, useState } from "react";
import { api, humanBytes } from "../lib/api";
import type { AppSettings, SecretsPresent, BackupReport, BackupHistoryItem, RestoreReport } from "../lib/api";

interface Props { settings: AppSettings; secrets: SecretsPresent | null; }

export function Backup({ settings, secrets }: Props) {
  const [running, setRunning] = useState(false);
  const [report, setReport] = useState<BackupReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [history, setHistory] = useState<BackupHistoryItem[]>([]);
  const [offsite, setOffsite] = useState<boolean>(false);
  const [nextTick, setNextTick] = useState<number | null>(null);
  const [, tick] = useState(0);

  // restore state
  const [confirmRestore, setConfirmRestore] = useState<string | null>(null);
  const [restoring, setRestoring] = useState(false);
  const [restoreResult, setRestoreResult] = useState<RestoreReport | null>(null);

  const loadHistory = () => api.backupHistory().then(setHistory).catch(() => setHistory([]));
  const loadOffsite = () => api.backupOffsiteStatus().then(setOffsite).catch(() => setOffsite(false));

  useEffect(() => { loadHistory(); loadOffsite(); }, []);
  useEffect(() => {
    const id = setInterval(() => tick(v => v + 1), 1000);
    return () => clearInterval(id);
  }, []);

  useEffect(() => {
    if (!settings.schedule_enabled || settings.schedule_minutes <= 0) { setNextTick(null); return; }
    const gap = settings.schedule_minutes * 60_000;
    const last = history[0]?.modified ? new Date(history[0].modified).getTime() : Date.now() - gap;
    let target = last + gap;
    if (target < Date.now()) target = Date.now() + 30_000;
    setNextTick(target);
    const timer = setInterval(async () => {
      if (Date.now() >= target && !running) { target = Date.now() + gap; setNextTick(target); await runBackup(); }
    }, 15_000);
    return () => clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings.schedule_enabled, settings.schedule_minutes, history.length]);

  async function runBackup() {
    setRunning(true); setError(null);
    try {
      const r = await api.backupNow();
      setReport(r);
      await loadHistory();
    } catch (e) { setError(String(e)); }
    finally { setRunning(false); }
  }

  async function doRestore(name: string) {
    setRestoring(true); setError(null); setRestoreResult(null);
    try {
      const r = await api.restoreBackup(name);
      setRestoreResult(r);
    } catch (e) { setError(String(e)); }
    finally { setRestoring(false); setConfirmRestore(null); }
  }

  const blocker = !settings.repo_local_path ? "Choose a local backup folder in Settings."
    : !secrets?.ftp ? "Set your SFTP password in Settings first."
    : null;

  const totalSize = history.reduce((a, b) => a + b.bytes, 0);

  return (
    <div className="view">
      <div className="section-head">
        <div className="section-title">BACKUP <span className="amber">OPS</span></div>
        <div className="section-meta">
          {settings.schedule_enabled
            ? `AUTO · every ${settings.schedule_minutes}m${nextTick ? ` · next ${countdown(nextTick)}` : ""}`
            : "MANUAL MODE"}
        </div>
      </div>

      {blocker && <div className="notice notice--warn">{blocker}</div>}
      {error && <div className="notice notice--warn">{error}</div>}

      {/* ── Off-site status banner ── */}
      <div className={`notice ${offsite ? "notice--good" : ""}`} style={{ marginBottom: 14 }}>
        <span className={`dot ${offsite ? "dot--green" : "dot--amber"}`} style={{ marginTop: 4 }} />
        <div>
          {offsite ? (
            <><strong>Off-site backup armed.</strong> Snapshots push to your private GitHub repo — safe even if this PC dies.</>
          ) : (
            <><strong>Local only — not yet off-site.</strong> Backups live on this machine only. Set a GitHub remote in Settings
              (or ask to arm it) so a dead drive can't take your world with it. Config secrets are auto-redacted before anything is pushed.</>
          )}
        </div>
      </div>

      {/* ── Stat strip ── */}
      <div className="stat-row" style={{ marginBottom: 14 }}>
        <div className="stat stat--live">
          <div className="stat-label">Snapshots</div>
          <div className="stat-value">{history.length}</div>
          <div className="stat-sub">keep {settings.backup_retention}</div>
        </div>
        <div className="stat">
          <div className="stat-label">Last Backup</div>
          <div className="stat-value" style={{ fontSize: 13 }}>{history[0]?.modified ?? "never"}</div>
          <div className="stat-sub">{history[0] ? humanBytes(history[0].bytes) : "run one below"}</div>
        </div>
        <div className="stat">
          <div className="stat-label">Total On Disk</div>
          <div className="stat-value" style={{ fontSize: 16 }}>{humanBytes(totalSize)}</div>
          <div className="stat-sub">{offsite ? "+ off-site" : "local only"}</div>
        </div>
        <div className={`stat ${settings.stop_before_backup ? "stat--good" : ""}`}>
          <div className="stat-label">Integrity Mode</div>
          <div className="stat-value" style={{ fontSize: 13 }}>
            {settings.stop_before_backup ? "STOP-FIRST" : "SAVE + PULL"}
          </div>
          <div className="stat-sub">{settings.stop_before_backup ? "guaranteed clean" : "save then pull"}</div>
        </div>
      </div>

      <div style={{ display: "flex", gap: 8, marginBottom: 14 }}>
        <button className="btn btn--primary" disabled={!!blocker || running || restoring} onClick={runBackup}>
          {running ? "⚙ WORKING…" : "▶ BACKUP NOW"}
        </button>
        <button className="btn" onClick={() => { loadHistory(); loadOffsite(); }}>↻ Refresh</button>
      </div>

      {report && (
        <div className="log" style={{ maxHeight: 220, marginBottom: 14 }}>
          <div className="log-line log-line--good">▸ {report.message}</div>
          <div className="log-line log-line--dim">
            worlds {report.worlds.length} · players {report.players} · configs {report.configs.length} · archives {report.archives.length}
          </div>
          {report.warnings.map((w, i) => <div key={i} className="log-line log-line--warn">⚠ {w}</div>)}
          <div className={`log-line log-line--${report.pushed ? "good" : report.committed ? "warn" : "dim"}`}>
            {report.pushed ? "▸ pushed to GitHub (off-site ✓)"
              : report.committed ? "▸ committed locally (no remote — local only)"
              : "▸ nothing new to commit"}
          </div>
        </div>
      )}

      {restoreResult && (
        <div className="notice notice--good" style={{ marginBottom: 14 }}>
          <div>
            <strong>Restore complete.</strong> {restoreResult.message}
            {restoreResult.warnings.map((w, i) => <div key={i} style={{ color: "var(--amber)", marginTop: 4 }}>⚠ {w}</div>)}
          </div>
        </div>
      )}

      {/* ── Snapshot browser ── */}
      <div className="panel">
        <div className="panel-head">
          <span className="panel-title">Snapshots — newest first</span>
          <span className="chip">{history.length}</span>
        </div>
        <div className="panel-body">
          {history.length === 0 ? (
            <div className="empty">No snapshots yet. Run your first backup above.</div>
          ) : (
            <div className="player-grid">
              {history.map((h, i) => (
                <div className="player-card" key={h.name} style={{ borderLeftColor: i === 0 ? "var(--green)" : "var(--amber)" }}>
                  <div className="player-head">
                    <div className="player-name" style={{ fontSize: 13 }}>{i === 0 ? "LATEST" : `#${history.length - i}`}</div>
                    <span className="player-level">{humanBytes(h.bytes)}</span>
                  </div>
                  <div className="player-row"><span>WHEN</span><strong style={{ fontSize: 10 }}>{h.modified}</strong></div>
                  <div className="player-row"><span>FILE</span><strong style={{ fontSize: 9 }}>{h.name}</strong></div>
                  <div className="player-actions">
                    {confirmRestore === h.name ? (
                      <>
                        <button className="btn btn--sm btn--danger" disabled={restoring}
                          onClick={() => doRestore(h.name)}>
                          {restoring ? "…" : "CONFIRM"}
                        </button>
                        <button className="btn btn--sm btn--ghost" disabled={restoring}
                          onClick={() => setConfirmRestore(null)}>Cancel</button>
                      </>
                    ) : (
                      <button className="btn btn--sm btn--ghost" style={{ color: "var(--amber)" }}
                        disabled={restoring || running}
                        onClick={() => { setConfirmRestore(h.name); setRestoreResult(null); }}>
                        ⤺ Restore
                      </button>
                    )}
                  </div>
                  {confirmRestore === h.name && (
                    <div style={{ fontSize: 10, color: "var(--red)", lineHeight: 1.4, marginTop: 6 }}>
                      Stops the server and overwrites the live world with this snapshot. If unsure,
                      hit BACKUP NOW first so the current state is saved.
                    </div>
                  )}
                </div>
              ))}
            </div>
          )}
        </div>
      </div>

      <div className="field-hint" style={{ marginTop: 12, lineHeight: 1.6 }}>
        Each snapshot is a <code>tar.gz</code> of that moment's world (Level.sav + all player saves), integrity-checked
        on capture. Config files are stored separately as text with passwords redacted. Restore stops the server, pushes
        the snapshot back over SFTP, and you restart from the panel.
      </div>
    </div>
  );
}

function countdown(target: number): string {
  const s = Math.max(0, Math.floor((target - Date.now()) / 1000));
  if (s >= 3600) return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
  if (s >= 60) return `${Math.floor(s / 60)}m ${s % 60}s`;
  return `${s}s`;
}
