use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use codex_usb_license::{anchor_sha256, bundle_anchor_schema, UsbFingerprint};
use serde::Serialize;

use crate::error::SignerError;
use crate::path_security::{
    bundle_path_on_drive, canonical_existing_path, ensure_no_reparse_components,
    ensure_tree_has_no_reparse_points, normalize_bundle_relative_path, normalize_drive_root,
    path_is_within, relative_path_from_drive,
};
use crate::windows::{drive_is_removable_ntfs, removable_ntfs_volumes, VolumeInfo};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanCandidate {
    pub drive_root: String,
    pub volume_serial: u32,
    pub file_system: String,
    pub bundle_relative_path: String,
    pub bundle_path: String,
    pub anchor_schema: u32,
    pub anchor_sha256: String,
}

#[derive(Debug, Clone)]
pub struct PhysicalObservation {
    pub volume: VolumeInfo,
    pub bundle_path: PathBuf,
    pub fingerprint: UsbFingerprint,
}

pub fn scan_removable_candidates() -> Result<Vec<ScanCandidate>, SignerError> {
    let mut candidates = Vec::new();
    for volume in removable_ntfs_volumes()? {
        candidates.extend(discover_candidates_on_volume(&volume)?);
    }
    Ok(candidates)
}

pub fn discover_candidates_on_volume(
    volume: &VolumeInfo,
) -> Result<Vec<ScanCandidate>, SignerError> {
    let root = PathBuf::from(&volume.drive_root);
    ensure_no_reparse_components(&root)?;
    let mut pending = vec![root.clone()];
    let mut visited = HashSet::new();
    let mut candidates = Vec::new();
    while let Some(directory) = pending.pop() {
        let canonical = canonical_existing_path(&directory)?;
        let key = canonical.to_string_lossy().to_ascii_lowercase();
        if !visited.insert(key) {
            continue;
        }
        if is_candidate_shape(&directory) {
            ensure_tree_has_no_reparse_points(&directory)?;
            let relative = relative_path_from_drive(&canonical, &volume.drive_root)?;
            let anchor_schema = bundle_anchor_schema(&canonical)?;
            let anchor_sha256 = anchor_sha256(&canonical)?;
            candidates.push(ScanCandidate {
                drive_root: volume.drive_root.clone(),
                volume_serial: volume.volume_serial,
                file_system: volume.file_system.clone(),
                bundle_relative_path: relative,
                bundle_path: canonical.display().to_string(),
                anchor_schema,
                anchor_sha256,
            });
        }
        for entry in fs::read_dir(&directory)
            .map_err(|error| SignerError::io("scanning removable volume", &directory, error))?
        {
            let entry = entry.map_err(|error| {
                SignerError::io("reading removable directory entry", &directory, error)
            })?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|error| {
                SignerError::io("reading removable entry attributes", &path, error)
            })?;
            if metadata_is_reparse(&metadata) {
                return Err(SignerError::PathRejected(format!(
                    "removable volume contains a reparse point: {}",
                    path.display()
                )));
            }
            if metadata.is_dir() {
                if is_protected_volume_metadata_dir(&root, &path) {
                    continue;
                }
                pending.push(path);
            }
        }
    }
    candidates.sort_by(|left, right| left.bundle_path.cmp(&right.bundle_path));
    Ok(candidates)
}

pub fn observe_target(
    drive: &str,
    bundle_relative_path: &str,
) -> Result<PhysicalObservation, SignerError> {
    let drive_root = normalize_drive_root(drive)?;
    let relative = normalize_bundle_relative_path(bundle_relative_path)?;
    let volume = drive_is_removable_ntfs(&drive_root)?;
    let requested = bundle_path_on_drive(&drive_root, &relative)?;
    ensure_no_reparse_components(&requested)?;
    let canonical = canonical_existing_path(&requested)?;
    let metadata = fs::symlink_metadata(&canonical)
        .map_err(|error| SignerError::io("reading target bundle", &canonical, error))?;
    if !metadata.is_dir() {
        return Err(SignerError::PathRejected(format!(
            "target bundle is not a directory: {}",
            requested.display()
        )));
    }
    ensure_no_reparse_components(&canonical)?;
    let canonical_drive = PathBuf::from(&drive_root);
    if !path_is_within(&canonical_drive, &canonical) {
        return Err(SignerError::PathRejected(format!(
            "target bundle escaped drive root: {}",
            canonical.display()
        )));
    }
    let actual_relative = relative_path_from_drive(&canonical, &drive_root)?;
    if actual_relative != relative {
        return Err(SignerError::PathRejected(format!(
            "target bundle relative path changed: requested {relative}, observed {actual_relative}"
        )));
    }
    let anchor_schema = bundle_anchor_schema(&canonical)?;
    let fingerprint = UsbFingerprint {
        volume_serial: volume.volume_serial,
        anchor_sha256: anchor_sha256(&canonical)?,
        bundle_relative_path: actual_relative,
        anchor_schema,
    };
    Ok(PhysicalObservation {
        volume,
        bundle_path: canonical,
        fingerprint,
    })
}

pub fn sole_candidate_is_target(
    candidates: &[ScanCandidate],
    drive: &str,
    bundle_relative_path: &str,
) -> Result<(), SignerError> {
    if candidates.is_empty() {
        return Err(SignerError::NoCandidates);
    }
    if candidates.len() != 1 {
        let details = candidates
            .iter()
            .map(|candidate| {
                format!(
                    "{}:{}",
                    candidate.drive_root, candidate.bundle_relative_path
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        return Err(SignerError::MultipleCandidates(details));
    }
    let expected_drive = normalize_drive_root(drive)?;
    let expected_relative = normalize_bundle_relative_path(bundle_relative_path)?;
    let candidate = &candidates[0];
    if !candidate.drive_root.eq_ignore_ascii_case(&expected_drive)
        || candidate.bundle_relative_path != expected_relative
    {
        return Err(SignerError::CandidateMismatch);
    }
    Ok(())
}

fn is_candidate_shape(directory: &Path) -> bool {
    ["manifest.json", "usb-manifest.json", "SHA256SUMS.txt"]
        .iter()
        .all(|name| directory.join(name).is_file())
        && directory.join("payload").is_dir()
}

fn is_protected_volume_metadata_dir(volume_root: &Path, path: &Path) -> bool {
    if path.parent() != Some(volume_root) {
        return false;
    }
    matches!(
        path.file_name()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .as_deref(),
        Some("system volume information") | Some("$recycle.bin") | Some("$extend")
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sole_candidate_requires_both_explicit_target_fields() {
        let candidate = ScanCandidate {
            drive_root: "E:\\".to_string(),
            volume_serial: 1,
            file_system: "NTFS".to_string(),
            bundle_relative_path: "codexusb".to_string(),
            bundle_path: "E:\\codexusb".to_string(),
            anchor_schema: 2,
            anchor_sha256: "a".repeat(64),
        };
        assert!(
            sole_candidate_is_target(std::slice::from_ref(&candidate), "E:", "CodexUsb").is_ok()
        );
        assert!(sole_candidate_is_target(&[candidate], "F:", "CodexUsb").is_err());
    }

    #[test]
    fn multiple_candidates_are_fail_closed() {
        let candidates = (0..2)
            .map(|index| ScanCandidate {
                drive_root: "E:\\".to_string(),
                volume_serial: index,
                file_system: "NTFS".to_string(),
                bundle_relative_path: format!("bundle{index}"),
                bundle_path: format!("E:\\bundle{index}"),
                anchor_schema: 2,
                anchor_sha256: "a".repeat(64),
            })
            .collect::<Vec<_>>();
        assert!(matches!(
            sole_candidate_is_target(&candidates, "E:", "bundle0"),
            Err(SignerError::MultipleCandidates(_))
        ));
    }

    #[test]
    fn protected_volume_metadata_directories_are_not_discovered() {
        let root = Path::new(r"F:\");
        assert!(is_protected_volume_metadata_dir(
            root,
            Path::new(r"F:\System Volume Information")
        ));
        assert!(is_protected_volume_metadata_dir(
            root,
            Path::new(r"F:\$RECYCLE.BIN")
        ));
        assert!(!is_protected_volume_metadata_dir(
            root,
            Path::new(r"F:\CodexUsb\System Volume Information")
        ));
    }
}
