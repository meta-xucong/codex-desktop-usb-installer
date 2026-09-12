//! Shared command pipeline for the USB license CLI and its native GUI.

#![allow(dead_code)]

pub mod bundle;
pub mod error;
pub mod key_blob;
pub mod media;
pub mod path_security;
pub mod transaction;
pub mod windows;

mod command;

pub use command::run_with_args;
pub use error::SignerError;
