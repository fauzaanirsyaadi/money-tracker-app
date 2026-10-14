// No commands are registered: platform behaviour is driven from Rust only.
// The app exposes the few calls it needs as its own commands.
const COMMANDS: &[&str] = &[];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
