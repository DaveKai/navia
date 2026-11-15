//! Navia's shared core, everything beneath the interface.
//!
//! The GPUI and SwiftUI apps stay thin by using only this crate's plain types.

pub mod artist_radio;
pub mod artwork;
pub mod audio;
pub mod audio_quality;
pub mod autoplay;
pub mod capabilities;
pub mod client;
pub mod connect;
pub mod deezer;
pub mod display;
pub mod downloads;
pub mod engine;
pub mod error;
pub mod lanyard;
pub mod library;
pub mod listen_along;
pub mod lyrics;
pub mod quality;
pub mod play_queue;
pub mod queue_order;
pub mod queue_sync;
pub mod recommend;
pub mod reporting;
pub mod session;
pub mod shares;
pub mod similar_artists;
pub mod smart_editor;
pub mod smart_rules;
pub mod artist_filters;
pub mod song_filters;
pub mod songcache;
pub mod stations;

pub use error::{Error, Result};
