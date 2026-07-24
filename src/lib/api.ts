import { invoke } from "@tauri-apps/api/core";

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

export const api = {
  getSettings: () => invoke<AppSettings>("get_settings"),
  setSettings: (settings: AppSettings) => invoke<void>("set_settings", { settings }),
  getSecretsPresent: () => invoke<SecretsPresent>("get_secrets_present"),
  setFtpPassword: (password: string) => invoke<void>("set_ftp_password", { password }),
  setAdminPassword: (password: string) => invoke<void>("set_admin_password", { password }),
  probeFtp: () => invoke<SftpProbe>("probe_ftp"),

  liveInfo: () => invoke<LiveInfo>("live_info"),
  liveMetrics: () => invoke<LiveMetrics>("live_metrics"),
  livePlayers: () => invoke<LivePlayer[]>("live_players"),
  liveAnnounce: (message: string) => invoke<void>("live_announce", { message }),
  liveSave: () => invoke<void>("live_save"),
  liveStop: () => invoke<void>("live_stop"),
  liveKick: (userid: string, message: string) =>
    invoke<void>("live_kick", { userid, message }),
  liveBan: (userid: string, message: string) =>
    invoke<void>("live_ban", { userid, message }),

  backupNow: () => invoke<BackupReport>("backup_now"),
  safeRestart: () => invoke<SafeRestartReport>("safe_restart"),
  backupHistory: () => invoke<BackupHistoryItem[]>("backup_history"),
  restoreBackup: (archiveName: string) =>
    invoke<RestoreReport>("restore_backup", { archiveName }),
  backupOffsiteStatus: () => invoke<boolean>("backup_offsite_status"),

  configLoad: () => invoke<PalConfigView>("config_load"),
  configSave: (updates: [string, string][]) =>
    invoke<string>("config_save", { updates }),

  modsList: () => invoke<ModEntry[]>("mods_list"),
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
