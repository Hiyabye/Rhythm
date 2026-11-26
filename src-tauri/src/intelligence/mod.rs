//! The embedding model, its downloads and the classification loop live in
//! `rhythm_core::intelligence` (#341); the app keeps only its Tauri commands.
pub use rhythm_core::intelligence::*;
pub mod commands;
