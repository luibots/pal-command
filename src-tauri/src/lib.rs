mod backup;
mod commands;
mod config_store;
mod gitrepo;
mod palconfig;
mod rcon_ctl;
mod rest;
mod savecheck;
mod sftp;

use commands::*;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            get_settings,
            set_settings,
            get_secrets_present,
            set_ftp_password,
            set_admin_password,
            probe_ftp,
            live_info,
            live_metrics,
            live_players,
            live_announce,
            live_save,
            live_shutdown,
            live_stop,
            live_kick,
            live_ban,
            backup_now,
            backup_history,
            restore_backup,
            backup_offsite_status,
            config_load,
            config_save,
            mods_list,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
