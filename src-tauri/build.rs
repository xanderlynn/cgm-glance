// Declares every app command so each window can be granted only the ones it uses
// (see capabilities/*.json). A command no capability grants can't be called at all.
const COMMANDS: &[&str] = &[
    "get_snapshot",
    "test_connection",
    "save_person",
    "update_person",
    "remove_person",
    "save_settings",
    "reset_badge_positions",
    "discard_drafts",
    "retry_person",
    "open_settings",
    "hide_popup",
    "quit_app",
    "acknowledge_alarms",
    "snooze_alarms",
    "set_quiet",
    "check_update",
    "install_update",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run tauri-build");
}
