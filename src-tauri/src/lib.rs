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

use std::sync::Arc;

use tauri::Manager;

use crate::commands::{ipc, AppState};

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
      let data_dir = app.path().app_data_dir()?;
      std::fs::create_dir_all(&data_dir)?;
      app.manage(AppState {
        catalog: Arc::new(catalog::Catalog::bundled()?),
        store: Arc::new(store::Store::open(data_dir.join("architect.sqlite"))?),
        secrets: Arc::new(secrets::KeyringStore::new()?),
      });
      Ok(())
    })
    .invoke_handler(tauri::generate_handler![
      ipc::get_settings,
      ipc::save_settings,
      ipc::get_criteria,
      ipc::set_api_key,
      ipc::clear_api_key,
      ipc::has_api_key,
      ipc::data_notice,
      ipc::ack_data_notice,
      ipc::health_check,
      ipc::local_model_status,
      ipc::start_describe,
      ipc::start_upload,
      ipc::get_session,
      ipc::list_sessions,
      ipc::update_brief,
      ipc::confirm_brief,
      ipc::run_decisions,
      ipc::retry,
      ipc::review,
      ipc::export_adrs,
      ipc::export_report,
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
