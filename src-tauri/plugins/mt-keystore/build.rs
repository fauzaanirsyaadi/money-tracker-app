// No commands are registered: key operations are reachable from Rust only,
// never from the WebView.
const COMMANDS: &[&str] = &[];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
