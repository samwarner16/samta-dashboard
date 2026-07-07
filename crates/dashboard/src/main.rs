#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Tauri frontend acts as a pure consumer of the Rust API & WebSockets.
// The Tauri app is optional (behind the `tauri` feature). The primary dashboard
// is the static HTML/JS served from crates/dashboard/frontend via scripts.

#[cfg(feature = "tauri")]
fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(not(feature = "tauri"))]
fn main() {
    println!("samta-dashboard (tauri stub)");
    println!("Use the web dashboard: ./scripts/start-dashboard.sh  or  python -m http.server 4173 --directory crates/dashboard/frontend");
    println!("Point it at the API with ?api=http://127.0.0.1:8080");
}
