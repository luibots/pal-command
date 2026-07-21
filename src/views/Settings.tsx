import { useEffect, useState } from "react";
import { api } from "../lib/api";
import type { AppSettings, SecretsPresent } from "../lib/api";

interface Props {
  settings: AppSettings | null;
  secrets: SecretsPresent | null;
  onSaved: () => void;
  initialSetup?: boolean;
}

const BLANK: AppSettings = {
  sftp_host: "",
  sftp_port: 22,
  sftp_user: "",
  save_games_path: "Pal/Saved/SaveGames/0",
  config_dir: "",
  rest_url: "",
  rest_enabled: false,
  rcon_host: "",
  rcon_port: 25575,
  rcon_enabled: false,
  repo_local_path: "",
  repo_remote: "",
  git_branch: "main",
  backup_retention: 20,
  stop_before_backup: false,
  schedule_enabled: false,
  schedule_minutes: 60,
};

export function Settings({ settings, secrets, onSaved, initialSetup }: Props) {
  const [s, setS] = useState<AppSettings>(settings ?? BLANK);
  const [ftpPw, setFtpPw] = useState("");
  const [adminPw, setAdminPw] = useState("");
  const [saveMsg, setSaveMsg] = useState<string | null>(null);
  const [probing, setProbing] = useState(false);
  const [probeMsg, setProbeMsg] = useState<string | null>(null);
  const [probeSample, setProbeSample] = useState<string[]>([]);

  useEffect(() => { if (settings) setS(settings); }, [settings]);

  const set = <K extends keyof AppSettings>(k: K, v: AppSettings[K]) =>
    setS(prev => ({ ...prev, [k]: v }));

  const save = async () => {
    setSaveMsg(null);
    try {
      await api.setSettings(s);
      if (ftpPw) { await api.setFtpPassword(ftpPw); setFtpPw(""); }
      if (adminPw) { await api.setAdminPassword(adminPw); setAdminPw(""); }
      setSaveMsg("Saved.");
      onSaved();
    } catch (e) { setSaveMsg(`Error: ${e}`); }
  };

  const probe = async () => {
    setProbing(true); setProbeMsg(null); setProbeSample([]);
    try {
      await api.setSettings(s);
      if (ftpPw) { await api.setFtpPassword(ftpPw); setFtpPw(""); }
      const r = await api.probeFtp();
      setProbeMsg(r.message);
      setProbeSample(r.sample);
      onSaved();
    } catch (e) { setProbeMsg(`Error: ${e}`); }
    finally { setProbing(false); }
  };

  const Wrapper = initialSetup ? (({ children }: { children: React.ReactNode }) => <>{children}</>)
    : (({ children }: { children: React.ReactNode }) => (
      <div className="view">
        <div className="section-head">
          <div className="section-title">SETTINGS</div>
        </div>
        {children}
      </div>
    ));

  return (
    <Wrapper>
      {saveMsg && (
        <div className={`notice ${saveMsg.startsWith("Error") ? "notice--warn" : "notice--good"}`}>
          {saveMsg}
        </div>
      )}

      <div className="panel" style={{ marginBottom: 12 }}>
        <div className="panel-head">
          <span className="panel-title">Host Havoc · File Access (SFTP)</span>
          <span className={`chip ${secrets?.ftp ? "chip--good" : "chip--warn"}`}>
            {secrets?.ftp ? "PASSWORD SAVED" : "NEEDED"}
          </span>
        </div>
        <div className="panel-body form-grid">
          <div className="field">
            <div className="field-label">SFTP Host</div>
            <input className="input" placeholder="e.g. 123.45.67.89"
              value={s.sftp_host} onChange={e => set("sftp_host", e.target.value)} />
            <div className="field-hint">From your panel's "SFTP Info" section</div>
          </div>
          <div className="field">
            <div className="field-label">Port</div>
            <input className="input input--num" type="number" min={1} max={65535}
              value={s.sftp_port} onChange={e => set("sftp_port", Number(e.target.value) || 22)} />
            <div className="field-hint">Host Havoc gave you 8825</div>
          </div>
          <div className="field">
            <div className="field-label">Username</div>
            <input className="input" placeholder="your panel username"
              value={s.sftp_user} onChange={e => set("sftp_user", e.target.value)} />
          </div>
          <div className="field">
            <div className="field-label">
              Password {secrets?.ftp && "(saved — leave blank to keep)"}
            </div>
            <input className="input" type="password" placeholder={secrets?.ftp ? "•••••••••••" : "panel password"}
              value={ftpPw} onChange={e => setFtpPw(e.target.value)} />
            <div className="field-hint">Stored in Windows Credential Manager only.</div>
          </div>
          <div className="field full">
            <div className="field-label">SaveGames path</div>
            <input className="input" value={s.save_games_path}
              onChange={e => set("save_games_path", e.target.value)} />
            <div className="field-hint">
              Relative to the SFTP root. WorldGUID subfolder is auto-detected.
            </div>
          </div>
          <div className="field full">
            <div className="field-label">Config dir (optional)</div>
            <input className="input" placeholder="auto-detect LinuxServer / WindowsServer"
              value={s.config_dir} onChange={e => set("config_dir", e.target.value)} />
          </div>
          <div className="full" style={{ display: "flex", gap: 8, alignItems: "center" }}>
            <button className="btn" onClick={probe} disabled={probing || !s.sftp_host || !s.sftp_user}>
              {probing ? "…" : "TEST CONNECTION"}
            </button>
            {probeMsg && (
              <span style={{ fontSize: 12, color: probeMsg.startsWith("Error") ? "var(--red)" : "var(--green)" }}>
                {probeMsg}
              </span>
            )}
          </div>
          {probeSample.length > 0 && (
            <div className="full">
              <div className="field-label" style={{ marginBottom: 6 }}>Found in SaveGames:</div>
              <div style={{ display: "flex", flexWrap: "wrap", gap: 4 }}>
                {probeSample.slice(0, 12).map(n => (
                  <span key={n} className="chip">{n}</span>
                ))}
              </div>
            </div>
          )}
          <div className="full field-hint">
            <strong>Requires Windows OpenSSH Client</strong> — usually pre-installed on Windows 11.
            If TEST CONNECTION says sftp.exe wasn't found: Settings → Apps → Optional Features →
            Add → OpenSSH Client.
          </div>
        </div>
      </div>

      <div className="panel" style={{ marginBottom: 12 }}>
        <div className="panel-head">
          <span className="panel-title">Live Control</span>
          <span className={`chip ${secrets?.admin ? "chip--good" : ""}`}>
            {secrets?.admin ? "ADMIN SAVED" : "OPTIONAL"}
          </span>
        </div>
        <div className="panel-body form-grid">
          <div className="full field-hint" style={{ lineHeight: 1.6 }}>
            The AdminPassword field below powers <strong>both</strong> RCON and REST. Set one
            channel (or both). PAL·COMMAND prefers REST when available (cleaner data, no
            broadcast bug); falls back to RCON otherwise.
          </div>

          <div className="field full">
            <div className="field-label">
              Admin Password {secrets?.admin && "(saved — leave blank to keep)"}
            </div>
            <input className="input" type="password"
              placeholder={secrets?.admin ? "•••••••••••" : "value of AdminPassword in PalWorldSettings.ini"}
              value={adminPw} onChange={e => setAdminPw(e.target.value)} />
            <div className="field-hint">
              Set this in the Palworld config (Config tab) too. Stored in Windows Credential Manager.
            </div>
          </div>

          {/* RCON */}
          <div className="field full" style={{ borderTop: "1px solid var(--line)", paddingTop: 12 }}>
            <label className="check">
              <input type="checkbox" checked={s.rcon_enabled}
                onChange={e => set("rcon_enabled", e.target.checked)} />
              Enable RCON (recommended — Host Havoc has a port allocated for you)
            </label>
          </div>
          <div className="field">
            <div className="field-label">RCON Host</div>
            <input className="input" placeholder="e.g. 123.45.67.89"
              value={s.rcon_host} onChange={e => set("rcon_host", e.target.value)} />
          </div>
          <div className="field">
            <div className="field-label">RCON Port</div>
            <input className="input input--num" type="number" min={1} max={65535}
              value={s.rcon_port} onChange={e => set("rcon_port", Number(e.target.value) || 25575)} />
            <div className="field-hint">Host Havoc gave you 30105</div>
          </div>
          <div className="full field-hint">
            Requires <code>RCONEnabled=True</code>, <code>RCONPort={s.rcon_port || 25575}</code>, and
            AdminPassword in your Palworld config (Config tab). Then restart the server.
          </div>

          {/* REST */}
          <div className="field full" style={{ borderTop: "1px solid var(--line)", paddingTop: 12 }}>
            <label className="check">
              <input type="checkbox" checked={s.rest_enabled}
                onChange={e => set("rest_enabled", e.target.checked)} />
              Enable REST API (needs a port opened by Host Havoc support — RCON is fine without it)
            </label>
          </div>
          <div className="field full">
            <div className="field-label">REST URL</div>
            <input className="input" placeholder="http://your-server:8212"
              value={s.rest_url} onChange={e => set("rest_url", e.target.value)} />
          </div>
        </div>
      </div>

      <div className="panel" style={{ marginBottom: 12 }}>
        <div className="panel-head"><span className="panel-title">Backup · GitHub</span></div>
        <div className="panel-body form-grid">
          <div className="field full">
            <div className="field-label">Local repo folder</div>
            <input className="input" placeholder="C:\\Users\\you\\palworld-backups"
              value={s.repo_local_path} onChange={e => set("repo_local_path", e.target.value)} />
            <div className="field-hint">
              PAL·COMMAND git-inits this folder on the first backup.
            </div>
          </div>
          <div className="field">
            <div className="field-label">Remote URL (optional)</div>
            <input className="input" placeholder="git@github.com:you/pal-backups.git"
              value={s.repo_remote} onChange={e => set("repo_remote", e.target.value)} />
            <div className="field-hint">Push uses your existing git credentials.</div>
          </div>
          <div className="field">
            <div className="field-label">Branch</div>
            <input className="input" value={s.git_branch}
              onChange={e => set("git_branch", e.target.value)} />
          </div>
          <div className="field">
            <div className="field-label">Retention (snapshots)</div>
            <input className="input input--num" type="number" min={1} max={500}
              value={s.backup_retention}
              onChange={e => set("backup_retention", Number(e.target.value) || 20)} />
          </div>
          <div className="field">
            <label className="check">
              <input type="checkbox" checked={s.stop_before_backup}
                onChange={e => set("stop_before_backup", e.target.checked)} />
              Stop server before backup (guaranteed integrity)
            </label>
            <div className="field-hint">
              Recommended for nightly backups. Otherwise a REST/RCON save is triggered first.
            </div>
          </div>
        </div>
      </div>

      <div className="panel" style={{ marginBottom: 20 }}>
        <div className="panel-head"><span className="panel-title">Schedule</span></div>
        <div className="panel-body form-grid">
          <div className="field">
            <label className="check">
              <input type="checkbox" checked={s.schedule_enabled}
                onChange={e => set("schedule_enabled", e.target.checked)} />
              Auto-backup on a schedule
            </label>
          </div>
          <div className="field">
            <div className="field-label">Every N minutes</div>
            <input className="input input--num" type="number" min={5} max={1440}
              value={s.schedule_minutes}
              onChange={e => set("schedule_minutes", Number(e.target.value) || 60)} />
            <div className="field-hint">Runs while the app is open.</div>
          </div>
        </div>
      </div>

      <div style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}>
        <button className="btn btn--primary" onClick={save}>SAVE SETTINGS</button>
      </div>
    </Wrapper>
  );
}
