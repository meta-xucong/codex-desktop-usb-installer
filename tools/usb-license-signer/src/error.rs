use std::path::Path;

use thiserror::Error;

use codex_usb_license::LicenseError;

#[derive(Debug, Error)]
pub enum SignerError {
    #[error("{0}")]
    HelpRequested(String),
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[allow(dead_code)]
    #[error("unsupported platform: the signer requires Windows")]
    UnsupportedPlatform,
    #[error("path rejected: {0}")]
    PathRejected(String),
    #[error("I/O error while {context} at {path}: {source}")]
    Io {
        context: &'static str,
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("license core error: {0}")]
    License(#[from] LicenseError),
    #[error("bundle verification failed: {0}")]
    BundleVerification(String),
    #[error("Authenticode verification failed for {label}: {detail}")]
    Authenticode { label: String, detail: String },
    #[error("no removable NTFS bundle candidate was found")]
    NoCandidates,
    #[error("multiple removable NTFS bundle candidates were found: {0}")]
    MultipleCandidates(String),
    #[error("requested bundle is not the sole scanned candidate")]
    CandidateMismatch,
    #[error("existing license cannot be overwritten: {0}")]
    ExistingLicense(String),
    #[error("another USB signing operation is already running on this signing machine")]
    SigningBusy,
    #[error("confirmation string is missing or not exact; expected: {expected}")]
    ConfirmationRequired { expected: String },
    #[error("key blob error: {0}")]
    KeyBlob(String),
    #[allow(dead_code)]
    #[error("drive or bundle changed after observation: {0}")]
    DriveChanged(String),
    #[error("signing result is unknown after the license replacement: {0}")]
    OutcomeUnknown(String),
    #[error("local report error: {0}")]
    Report(String),
}

impl SignerError {
    pub fn io(context: &'static str, path: &Path, source: std::io::Error) -> Self {
        Self::Io {
            context,
            path: path.display().to_string(),
            source,
        }
    }

    pub fn bundle(message: impl Into<String>) -> Self {
        Self::BundleVerification(message.into())
    }
}
