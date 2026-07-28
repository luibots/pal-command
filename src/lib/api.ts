import { invoke } from "@tauri-apps/api/core";

const demoMode =
  typeof window !== "undefined" &&
  new URLSearchParams(window.location.search).get("demo") === "1";

export interface AppSettings {
  sftp_host: string;
  sftp_port: number;
  sftp_user: string;
  save_games_path: string;
  config_dir: string;
  rest_url: string;
  rest_enabled: boolean;
  rcon_host: string;
  rcon_port: number;
  rcon_enabled: boolean;
  repo_local_path: string;
  repo_remote: string;
  git_branch: string;
  backup_retention: number;
  stop_before_backup: boolean;
  schedule_enabled: boolean;
  schedule_minutes: number;
}

export interface SecretsPresent { ftp: boolean; admin: boolean; migrated: boolean; }

export interface SftpProbe { ok: boolean; message: string; sample: string[]; }

export interface LiveInfo {
  source: "rest" | "rcon";
  servername?: string;
  version?: string;
  worldguid?: string;
  raw?: string;
}

export interface LiveMetrics {
  source: "rest" | "rcon";
  currentplayernum?: number;
  maxplayernum?: number;
  serverfps?: number;
  serverframetime?: number;
  uptime?: number;
  basecampnum?: number;
  days?: number;
}

export interface LivePlayer {
  name?: string;
  accountName?: string;
  playerId?: string;
  userId?: string;
  ip?: string;
  ping?: number;
  location_x?: number;
  location_y?: number;
  level?: number;
  building_count?: number;
}

export interface ArchiveInfo { name: string; bytes: number; }

export interface BackupReport {
  timestamp: string;
  worlds: string[];
  players: number;
  configs: string[];
  archives: ArchiveInfo[];
  warnings: string[];
  committed: boolean;
  pushed: boolean;
  message: string;
}

export interface BackupHistoryItem { name: string; bytes: number; modified: string; }

export interface SafeRestartReport {
  backup: BackupReport;
  player_checks: number;
  waittime: number;
  recovery_seconds: number;
}

export interface RestoreReport {
  world_folder: string;
  files_restored: number;
  server_stopped: boolean;
  warnings: string[];
  message: string;
}

export interface PalConfigView {
  pairs: [string, string][];
  source: string;
}

export interface ModEntry { name: string; kind: string; path: string; }

const demoSettings: AppSettings = {
  sftp_host: "palworld-admin.invalid",
  sftp_port: 22,
  sftp_user: "demo-admin",
  save_games_path: "/Pal/Saved/SaveGames/0",
  config_dir: "/Pal/Saved/Config/WindowsServer",
  rest_url: "http://palworld-admin.invalid:8212",
  rest_enabled: true,
  rcon_host: "",
  rcon_port: 25575,
  rcon_enabled: false,
  repo_local_path: "C:\\Palworld\\WorldBackups",
  repo_remote: "",
  git_branch: "main",
  backup_retention: 14,
  stop_before_backup: false,
  schedule_enabled: true,
  schedule_minutes: 30,
};

function demoResult<T>(command: string): Promise<T> {
  const now = Date.now();
  const fixtures: Record<string, unknown> = {
    get_settings: demoSettings,
    get_secrets_present: { ftp: true, admin: true, migrated: true },
    live_info: {
      source: "rest",
      servername: "AYEGUILD // SANITIZED DEMO",
      version: "v1.0.1",
      worldguid: "demo0001",
    },
    live_metrics: {
      source: "rest",
      currentplayernum: 3,
      maxplayernum: 32,
      serverfps: 59.7,
      serverframetime: 16.75,
      uptime: 268_740,
      basecampnum: 8,
      days: 674,
    },
    live_players: [
      {
        name: "Builder",
        userId: "demo-player-01",
        ping: 31,
        location_x: -168_080,
        location_y: 210_900,
        level: 44,
        building_count: 186,
      },
      {
        name: "Scout",
        userId: "demo-player-02",
        ping: 54,
        location_x: -91_720,
        location_y: 137_760,
        level: 39,
        building_count: 74,
      },
      {
        name: "Rancher",
        userId: "demo-player-03",
        ping: 42,
        location_x: -45_180,
        location_y: 301_700,
        level: 41,
        building_count: 121,
      },
    ],
    backup_history: [
      {
        name: "world-20260727-2118.zip",
        bytes: 2_340_000,
        modified: new Date(now - 12 * 60_000).toISOString(),
      },
      {
        name: "world-20260727-2048.zip",
        bytes: 2_310_000,
        modified: new Date(now - 42 * 60_000).toISOString(),
      },
    ],
    backup_offsite_status: true,
    config_load: { pairs: [], source: "sanitized demo" },
    mods_list: [],
  };
  if (command in fixtures) return Promise.resolve(fixtures[command] as T);
  return Promise.reject(new Error("Write operations are disabled in screenshot demo mode."));
}

function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  return demoMode ? demoResult<T>(command) : invoke<T>(command, args);
}

export const api = {
  getSettings: () => call<AppSettings>("get_settings"),
  setSettings: (settings: AppSettings) => call<void>("set_settings", { settings }),
  getSecretsPresent: () => call<SecretsPresent>("get_secrets_present"),
  setFtpPassword: (password: string) => call<void>("set_ftp_password", { password }),
  setAdminPassword: (password: string) => call<void>("set_admin_password", { password }),
  probeFtp: () => call<SftpProbe>("probe_ftp"),

  liveInfo: () => call<LiveInfo>("live_info"),
  liveMetrics: () => call<LiveMetrics>("live_metrics"),
  livePlayers: () => call<LivePlayer[]>("live_players"),
  liveAnnounce: (message: string) => call<void>("live_announce", { message }),
  liveSave: () => call<void>("live_save"),
  liveStop: () => call<void>("live_stop"),
  liveKick: (userid: string, message: string) =>
    call<void>("live_kick", { userid, message }),
  liveBan: (userid: string, message: string) =>
    call<void>("live_ban", { userid, message }),

  backupNow: () => call<BackupReport>("backup_now"),
  safeRestart: () => call<SafeRestartReport>("safe_restart"),
  backupHistory: () => call<BackupHistoryItem[]>("backup_history"),
  restoreBackup: (archiveName: string) =>
    call<RestoreReport>("restore_backup", { archiveName }),
  backupOffsiteStatus: () => call<boolean>("backup_offsite_status"),

  configLoad: () => call<PalConfigView>("config_load"),
  configSave: (updates: [string, string][]) =>
    call<string>("config_save", { updates }),

  modsList: () => call<ModEntry[]>("mods_list"),
};

export function humanBytes(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(2)} GB`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)} MB`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(1)} KB`;
  return `${n} B`;
}

export function humanUptime(seconds: number | undefined): string {
  if (!seconds || seconds <= 0) return "—";
  const d = Math.floor(seconds / 86400);
  const h = Math.floor((seconds % 86400) / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}
