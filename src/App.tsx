import { useEffect, useState } from "react";
import { api } from "./lib/api";
import type { AppSettings, SecretsPresent, LiveInfo, LiveMetrics } from "./lib/api";
import { Dashboard } from "./views/Dashboard";
import { Backup } from "./views/Backup";
import { Config } from "./views/Config";
import { Settings } from "./views/Settings";
import "./App.css";

type Tab = "DASHBOARD" | "BACKUP" | "CONFIG" | "SETTINGS";

export default function App() {
  const [tab, setTab] = useState<Tab>("DASHBOARD");
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [secrets, setSecrets] = useState<SecretsPresent | null>(null);
  const [info, setInfo] = useState<LiveInfo | null>(null);
  const [metrics, setMetrics] = useState<LiveMetrics | null>(null);
  const [liveOk, setLiveOk] = useState<boolean | null>(null);

  const loadSettings = () => api.getSettings().then(setSettings).catch(console.error);
  const loadSecrets = () => api.getSecretsPresent().then(setSecrets).catch(console.error);

  useEffect(() => {
    loadSettings();
    loadSecrets();
  }, []);

  const liveConfigured =
    !!secrets?.admin &&
    ((settings?.rest_enabled && !!settings.rest_url) ||
      (settings?.rcon_enabled && !!settings.rcon_host));

  useEffect(() => {
    if (!liveConfigured) {
      setLiveOk(null); setInfo(null); setMetrics(null);
      return;
    }
    let alive = true;
    const poll = async () => {
      try {
        const [i, m] = await Promise.all([api.liveInfo(), api.liveMetrics()]);
        if (!alive) return;
        setInfo(i); setMetrics(m); setLiveOk(true);
      } catch {
        if (!alive) return;
        setLiveOk(false); setInfo(null); setMetrics(null);
      }
    };
    poll();
    const id = setInterval(poll, 8_000);
    return () => { alive = false; clearInterval(id); };
  }, [liveConfigured]);

  const needsSetup = settings && (!settings.sftp_host || !settings.sftp_user || !secrets?.ftp);

  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">
          <span className="brand-p">PAL</span>
          <span className="brand-dot">·</span>
          <span className="brand-c">COMMAND</span>
          <span className="brand-tag">v0.1</span>
        </div>
        <nav className="tabs">
          {(["DASHBOARD", "BACKUP", "CONFIG", "SETTINGS"] as Tab[]).map(t => (
            <button
              key={t}
              className={`tab ${tab === t ? "tab--active" : ""}`}
              onClick={() => setTab(t)}
            >{t}</button>
          ))}
        </nav>
        <div className="topbar-right">
          {info?.servername && <span className="top-server">{info.servername}</span>}
          <span className="top-status">
            <span className={`dot ${
              liveOk === true ? "dot--green dot--pulse"
              : liveOk === false ? "dot--red"
              : "dot--dim"
            }`} />
            {liveOk === true ? `${metrics?.currentplayernum ?? 0} / ${metrics?.maxplayernum ?? "?"} online`
              : liveOk === false ? `${info?.source ?? "live"} unreachable`
              : liveConfigured ? "connecting…"
              : "live control off"}
          </span>
        </div>
      </header>

      <main className="content">
        {!settings ? (
          <div className="loading">LOADING…</div>
        ) : needsSetup ? (
          <FirstRun onDone={() => { loadSettings(); loadSecrets(); }} />
        ) : (
          <>
            {tab === "DASHBOARD" && (
              <Dashboard
                info={info} metrics={metrics} liveOk={liveOk}
                settings={settings} secrets={secrets}
              />
            )}
            {tab === "BACKUP" && <Backup settings={settings} secrets={secrets} />}
            {tab === "CONFIG" && <Config settings={settings} secrets={secrets} />}
            {tab === "SETTINGS" && (
              <Settings
                settings={settings} secrets={secrets}
                onSaved={() => { loadSettings(); loadSecrets(); }}
              />
            )}
          </>
        )}
      </main>

      <footer className="statusbar">
        <span className="statusbar-item">
          <span className={`dot ${secrets?.ftp ? "dot--green" : "dot--dim"}`} />
          SFTP {secrets?.ftp ? "READY" : "NOT SET"}
        </span>
        <span className="statusbar-item">
          <span className={`dot ${secrets?.admin ? "dot--green" : "dot--dim"}`} />
          LIVE {secrets?.admin ? "READY" : "NOT SET"}
        </span>
        {info?.source && (
          <span className="statusbar-item">via {info.source.toUpperCase()}</span>
        )}
        {settings?.repo_local_path && (
          <span className="statusbar-item" style={{ marginLeft: "auto" }}>
            REPO · {settings.repo_local_path}
          </span>
        )}
      </footer>
    </div>
  );
}

function FirstRun({ onDone }: { onDone: () => void }) {
  return (
    <div className="setup-wrap">
      <div className="setup-title">
        <span className="amber">TACTICAL SETUP</span>
      </div>
      <p className="setup-sub">
        Point PAL·COMMAND at your Host Havoc Palworld server. Grab the SFTP address from
        your game panel's "SFTP Info" section. Passwords stay in Windows Credential Manager
        — never on disk.
      </p>
      <Settings settings={null} secrets={null} onSaved={onDone} initialSetup />
    </div>
  );
}
