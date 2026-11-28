fn main() {
    // The app's commands get a permission each, which only the picker's capability grants.
    let commands = tauri_build::AppManifest::new().commands(&[
        "startup",
        "connect",
        "open_server",
        "rename_server",
        "remove_server",
        "cancel_opening",
        "check_for_updates",
    ]);
    if let Err(error) =
        tauri_build::try_build(tauri_build::Attributes::new().app_manifest(commands))
    {
        panic!("the Tauri build failed: {error:#}");
    }
}
