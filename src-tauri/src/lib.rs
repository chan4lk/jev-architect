pub mod model;

pub mod brief;
pub mod catalog;
pub mod commands;
pub mod docs;
pub mod jev;
pub mod ollama;
pub mod pipeline;
pub mod questions;
pub mod render;
pub mod rules;
pub mod scoring;
pub mod secrets;
pub mod store;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  tauri::Builder::default()
    .plugin(tauri_plugin_dialog::init())
    .setup(|app| {
      if cfg!(debug_assertions) {
        app.handle().plugin(
          tauri_plugin_log::Builder::default()
            .level(log::LevelFilter::Info)
            .build(),
        )?;
      }
      Ok(())
    })
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
