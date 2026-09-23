//! Shared command pipeline for the USB license CLI and its native GUI.

#![allow(dead_code)]

pub mod bundle;
pub mod error;
pub mod key_blob;
pub mod media;
pub mod path_security;
pub mod sub2api;
pub mod transaction;
pub mod windows;

mod command;

pub use command::{
    provision_default_portable_vault, provision_default_portable_vault_for_binding, run_with_args,
    run_with_portable_password, run_with_portable_password_and_account,
};
pub use error::SignerError;
