import { useEffect, useMemo, useState } from "react";
import { api } from "../lib/api";
import type { AppSettings, SecretsPresent, PalConfigView } from "../lib/api";

interface Props { settings: AppSettings; secrets: SecretsPresent | null; }

// A curated set of the most-tuned keys so the UI doesn't dump 100+ raw rows on the user.
// Everything else is available in the "advanced" table below.
const FEATURED: { key: string; label: string; hint?: string; kind: "text" | "bool" | "num" }[] = [
  { key: "ServerName", label: "Server Name", kind: "text" },
  { key: "ServerDescription", label: "Description", kind: "text" },
  { key: "ServerPassword", label: "Join Password", kind: "text", hint: "empty = no password" },
  { key: "AdminPassword", label: "Admin Password", kind: "text", hint: "Also used for RCON + REST auth" },
  { key: "ServerPlayerMaxNum", label: "Max Players", kind: "num" },
  { key: "PublicPort", label: "Public Port", kind: "num" },
  { key: "RESTAPIEnabled", label: "REST API On", kind: "bool" },
  { key: "RESTAPIPort", label: "REST API Port", kind: "num" },
  { key: "RCONEnabled", label: "RCON On", kind: "bool", hint: "Deprecated — prefer REST" },
  { key: "RCONPort", label: "RCON Port", kind: "num" },
  { key: "bIsPvP", label: "PvP", kind: "bool" },
  { key: "DeathPenalty", label: "Death Penalty", kind: "text", hint: "None / Item / ItemAndEquipment / All" },
  { key: "ExpRate", label: "XP Rate", kind: "num" },
  { key: "PalCaptureRate", label: "Pal Capture Rate", kind: "num" },
  { key: "DayTimeSpeedRate", label: "Day Speed", kind: "num" },
  { key: "NightTimeSpeedRate", label: "Night Speed", kind: "num" },
];

export function Config({ settings, secrets }: Props) {
  const [view, setView] = useState<PalConfigView | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saveMsg, setSaveMsg] = useState<string | null>(null);
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [showAll, setShowAll] = useState(false);

  const load = async () => {
    setLoading(true); setError(null); setSaveMsg(null); setDrafts({});
    try {
      const v = await api.configLoad();
      setView(v);
    } catch (e) { setError(String(e)); }
    finally { setLoading(false); }
  };

  useEffect(() => {
    if (secrets?.ftp && settings.sftp_host) load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [secrets?.ftp, settings.sftp_host]);

  const original = useMemo(() => {
    const m = new Map<string, string>();
    view?.pairs.forEach(([k, v]) => m.set(k, v));
    return m;
  }, [view]);

  const changes = useMemo(() => {
    return Object.entries(drafts).filter(([k, v]) => original.get(k) !== v);
  }, [drafts, original]);

  const setDraft = (k: string, v: string) => setDrafts(d => ({ ...d, [k]: v }));

  const readVal = (k: string): string => drafts[k] ?? original.get(k) ?? "";

  const save = async () => {
    setSaveMsg(null); setError(null);
    try {
      const msg = await api.configSave(changes as [string, string][]);
      setSaveMsg(msg);
      await load();
    } catch (e) { setError(String(e)); }
  };

  if (!secrets?.ftp) {
    return (
      <div className="view">
        <div className="notice notice--warn">
          Set your FTP password in Settings to load the server config.
        </div>
      </div>
    );
  }

  return (
    <div className="view">
      <div className="section-head">
        <div className="section-title">SERVER <span className="amber">CONFIG</span></div>
        <div className="section-meta">
          {view?.source ?? (loading ? "LOADING…" : "—")}
        </div>
      </div>

      <div className="notice" style={{ marginBottom: 14 }}>
        Palworld only reads settings at boot — <strong>stop the server before saving</strong>, then
        restart from the Host Havoc panel. Every save creates a timestamped <code>.bak-*</code>
        alongside the file so you can revert. A single malformed value silently reverts ALL
        settings to defaults; PAL·COMMAND validates the file before uploading it.
      </div>

      {error && <div className="notice notice--warn">{error}</div>}
      {saveMsg && <div className="notice notice--good">{saveMsg}</div>}

      <div style={{ display: "flex", gap: 8, marginBottom: 14 }}>
        <button className="btn" onClick={load} disabled={loading}>
          {loading ? "…" : "↻ Reload"}
        </button>
        <button
          className="btn btn--primary"
          onClick={save}
          disabled={changes.length === 0}
        >
          Save {changes.length ? `${changes.length} change${changes.length > 1 ? "s" : ""}` : ""}
        </button>
        <button
          className="btn btn--ghost"
          onClick={() => setDrafts({})}
          disabled={changes.length === 0}
        >Discard</button>
        <div style={{ marginLeft: "auto", display: "flex", alignItems: "center" }}>
          <label className="check">
            <input type="checkbox" checked={showAll} onChange={e => setShowAll(e.target.checked)} />
            show all {view ? `(${view.pairs.length})` : ""}
          </label>
        </div>
      </div>

      {view && (
        <>
          <div className="panel" style={{ marginBottom: 12 }}>
            <div className="panel-head"><span className="panel-title">Featured Settings</span></div>
            <div className="panel-body" style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "12px 16px" }}>
              {FEATURED.map(f => {
                const cur = readVal(f.key);
                const changed = drafts[f.key] !== undefined && drafts[f.key] !== original.get(f.key);
                if (!original.has(f.key)) return null;
                return (
                  <div className="field" key={f.key}>
                    <div className="field-label" style={{ color: changed ? "var(--amber)" : undefined }}>
                      {f.label} {changed && "●"}
                    </div>
                    {f.kind === "bool" ? (
                      <select className="select" value={cur} onChange={e => setDraft(f.key, e.target.value)}>
                        <option value="True">True</option>
                        <option value="False">False</option>
                      </select>
                    ) : (
                      <input
                        className="input"
                        value={cur}
                        onChange={e => setDraft(f.key, e.target.value)}
                        inputMode={f.kind === "num" ? "decimal" : "text"}
                      />
                    )}
                    {f.hint && <div className="field-hint">{f.hint}</div>}
                  </div>
                );
              })}
            </div>
          </div>

          {showAll && (
            <div className="panel">
              <div className="panel-head">
                <span className="panel-title">All Options ({view.pairs.length})</span>
              </div>
              <div style={{ maxHeight: 500, overflow: "auto" }}>
                <table className="table">
                  <thead><tr><th>Key</th><th>Value</th></tr></thead>
                  <tbody>
                    {view.pairs.map(([k]) => {
                      const cur = readVal(k);
                      const changed = drafts[k] !== undefined && drafts[k] !== original.get(k);
                      return (
                        <tr key={k}>
                          <td className="mono" style={{ fontSize: 11, color: changed ? "var(--amber)" : undefined }}>
                            {k}
                          </td>
                          <td style={{ width: "60%" }}>
                            <input
                              className="input"
                              style={{ padding: "3px 8px", fontSize: 11 }}
                              value={cur}
                              onChange={e => setDraft(k, e.target.value)}
                            />
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            </div>
          )}
        </>
      )}
    </div>
  );
}
