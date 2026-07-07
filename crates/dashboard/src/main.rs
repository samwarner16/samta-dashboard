#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Tauri frontend acts as a pure consumer of the Rust API & WebSockets.
// TODO: Implement window configuration and IPC if needed.
fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
