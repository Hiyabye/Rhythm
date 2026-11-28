//! The embedding model, its downloads and the classification loop live in
//! `rhythm_core::intelligence` (#341) behind `App::ai()` (#342); the app keeps
//! only its Tauri commands.
pub mod commands;
