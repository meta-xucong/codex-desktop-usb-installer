use std::fs;
use std::path::{Path, PathBuf};

use crate::error::SignerError;

/// Normalize a drive-only argument.  UNC, volume-GUID and drive-relative
/// paths are intentionally not accepted by the signer interface.
pub fn normalize_drive_root(input: &str) -> Result<String, SignerError> {
    let raw = input.trim().replace('/', "\\");
    let raw = raw
        .strip_prefix(r"\\?\")
        .or_else(|| raw.strip_prefix(r"\.\"))
        .unwrap_or(&raw);
    let bytes = raw.as_bytes();
    if bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Ok(format!("{}:\\", (bytes[0] as char).to_ascii_uppercase()));
    }
    if bytes.len() == 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\' {
        return Ok(format!("{}:\\", (bytes[0] as char).to_ascii_uppercase()));
    }
    Err(SignerError::PathRejected(format!(
        "drive must be a local drive root such as E:, got {input}"
    )))
}

/// Normalize a bundle-relative path to lower-case slash-separated form.
pub fn normalize_bundle_relative_path(input: &str) -> Result<String, SignerError> {
    let raw = input.trim().replace('\\', "/");
    if raw.is_empty()
        || raw.starts_with('/')
        || raw.contains(':')
        || raw.contains('\0')
        || raw
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(SignerError::PathRejected(format!(
            "bundle-relative path contains an absolute, empty, or traversal component: {input}"
        )));
    }
    Ok(raw.to_ascii_lowercase())
}

pub fn bundle_path_on_drive(drive_root: &str, relative: &str) -> Result<PathBuf, SignerError> {
    let root = normalize_drive_root(drive_root)?;
    let relative = normalize_bundle_relative_path(relative)?;
    Ok(PathBuf::from(root).join(relative.replace('/', "\\")))
}

pub fn drive_root_from_absolute_path(path: &Path) -> Result<String, SignerError> {
    let text = path.to_string_lossy().replace('/', "\\");
    let bytes = text.as_bytes();
    if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' || bytes[2] != b'\\' {
        return Err(SignerError::PathRejected(format!(
            "path must be an absolute local DOS path: {}",
            path.display()
        )));
    }
    normalize_drive_root(&text[..3])
}

/// A lexical containment check used only after the caller has canonicalized
/// existing paths.  Windows comparisons are case-insensitive.
pub fn path_is_within(base: &Path, candidate: &Path) -> bool {
    let base = normalize_path_text(base);
    let candidate = normalize_path_text(candidate);
    let (base, candidate) = if cfg!(windows) {
        (base.to_ascii_lowercase(), candidate.to_ascii_lowercase())
    } else {
        (base, candidate)
    };
    candidate == base || candidate.strip_prefix(&(base + "/")).is_some()
}

pub fn relative_path_from_drive(path: &Path, drive_root: &str) -> Result<String, SignerError> {
    let root = normalize_drive_root(drive_root)?;
    let path_text = normalize_path_text(path);
    let root_text = normalize_path_text(Path::new(&root));
    let comparison_path = if cfg!(windows) {
        path_text.to_ascii_lowercase()
    } else {
        path_text.clone()
    };
    let comparison_root = if cfg!(windows) {
        root_text.to_ascii_lowercase()
    } else {
        root_text.clone()
    };
    if !comparison_path.starts_with(&comparison_root) {
        return Err(SignerError::PathRejected(format!(
            "bundle is outside drive root: {}",
            path.display()
        )));
    }
    let relative = path_text[root_text.len()..].trim_matches('/');
    normalize_bundle_relative_path(relative)
}

fn normalize_path_text(path: &Path) -> String {
    let mut value = path.to_string_lossy().replace('\\', "/");
    if value.len() >= 4
        && (value[..4].eq_ignore_ascii_case("//?/") || value[..4].eq_ignore_ascii_case("//./"))
    {
        value = value[4..].to_string();
    }
    while value.len() > 1 && value.ends_with('/') {
        value.pop();
    }
    value
}

#[cfg(target_os = "windows")]
fn metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(target_os = "windows"))]
fn metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

pub fn path_has_reparse_point(path: &Path) -> Result<bool, SignerError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata_is_reparse(&metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(SignerError::io("reading path attributes", path, error)),
    }
}

/// Check every existing component and refuse links/reparse points before any
/// path is used for a read or write.  Nonexistent output leaves are allowed,
/// but their nearest existing ancestor is still checked.
pub fn ensure_no_reparse_components(path: &Path) -> Result<(), SignerError> {
    // `Path::exists()` is false for a broken symlink. Start at the requested
    // path itself so a broken link cannot become an unchecked write target;
    // `symlink_metadata` remains authoritative for both existing and missing
    // leaves.
    let mut current = path;
    loop {
        if path_has_reparse_point(current)? {
            return Err(SignerError::PathRejected(format!(
                "reparse point, junction, or symlink in path: {}",
                current.display()
            )));
        }
        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent;
    }
    Ok(())
}

/// Recursively inspect a bundle without following a directory reparse point.
pub fn ensure_tree_has_no_reparse_points(root: &Path) -> Result<(), SignerError> {
    if path_has_reparse_point(root)? {
        return Err(SignerError::PathRejected(format!(
            "bundle root is a reparse point: {}",
            root.display()
        )));
    }
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .map_err(|error| SignerError::io("enumerating bundle", &directory, error))?
        {
            let entry = entry.map_err(|error| {
                SignerError::io("reading bundle directory entry", &directory, error)
            })?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|error| {
                SignerError::io("reading bundle entry attributes", &path, error)
            })?;
            if metadata_is_reparse(&metadata) {
                return Err(SignerError::PathRejected(format!(
                    "bundle contains a reparse point, junction, or symlink: {}",
                    path.display()
                )));
            }
            if metadata.is_dir() {
                pending.push(path);
            }
        }
    }
    Ok(())
}

pub fn canonical_existing_path(path: &Path) -> Result<PathBuf, SignerError> {
    fs::canonicalize(path).map_err(|error| SignerError::io("canonicalizing bundle", path, error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extended_local_paths_compare_with_drive_paths() {
        assert!(path_is_within(
            Path::new("E:\\"),
            Path::new(r"\\?\E:\CodexUsb")
        ));
        assert!(!path_is_within(
            Path::new("E:\\"),
            Path::new(r"\\?\F:\CodexUsb")
        ));
    }

    #[test]
    fn drive_and_bundle_inputs_reject_unc_and_traversal() {
        assert!(normalize_drive_root(r"\\server\share").is_err());
        assert!(normalize_bundle_relative_path("CodexUsb/../other").is_err());
    }
}
