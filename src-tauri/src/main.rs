// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod alarms;
mod commands;
mod config;
mod icon;
mod model;
mod poller;
mod quiet;
mod secrets;
#[cfg(test)]
mod sim;
mod sound;
mod sources;
mod state;
mod ui;
mod updater;

use tauri::Manager;

fn demo_requested() -> bool {
    std::env::args().any(|a| a == "--demo")
        || std::env::var("CGM_GLANCE_DEMO").is_ok_and(|v| v == "1")
}

fn main() {
    let demo = demo_requested();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            ui::open_settings(app, None)
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::test_connection,
            commands::save_person,
            commands::update_person,
            commands::remove_person,
            commands::save_settings,
            commands::reset_badge_positions,
            commands::discard_drafts,
            commands::retry_person,
            commands::open_settings,
            commands::hide_popup,
            commands::quit_app,
            commands::acknowledge_alarms,
            commands::snooze_alarms,
            commands::set_quiet,
            commands::check_update,
            commands::install_update,
        ])
        .setup(move |app| {
            // Tray-only app: no Dock icon on macOS.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle().clone();
            let dir = app.path().app_config_dir()?;
            let path = config::file_path(&dir, demo);
            let mut cfg = config::load(&path);
            if demo && cfg.people.is_empty() {
                cfg.people = sources::demo::seed_people();
                cfg.settings.demo_fast = true;
                cfg.settings.badge.enabled = true;
                config::save(&path, &cfg)?;
            }
            let first_run = cfg.people.is_empty();
            app.manage(state::AppState::new(cfg, path, demo));
            poller::start_all(&handle);
            ui::start_ticker(&handle);
            updater::start(&handle);
            ui::refresh(&handle);
            if first_run {
                ui::open_settings(&handle, Some("people"));
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to start CGM Glance")
        .run(|_app, event| {
            // Closing the settings window must not quit a tray app.
            if let tauri::RunEvent::ExitRequested {
                api, code: None, ..
            } = event
            {
                api.prevent_exit();
            }
        });
}
