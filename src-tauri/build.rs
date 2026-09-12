use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

const BASE_SOURCE_RELATIVE: &str = "../installer-pack/manifest/skill-audit.json";
const BASE_FILE_MANIFEST_RELATIVE: &str = "../installer-pack/file-manifest.json";
const BASE_CHECKSUMS_RELATIVE: &str = "../installer-pack/checksums.sha256";
const BASE_RESOURCE_RELATIVE: &str = "resources/skill_repo_base_manifest.json";
const BASE_SOURCE_PATH: &str = "manifest/skill-audit.json";
const BASE_RESOURCE_SOURCE: &str = "installer-pack/manifest/skill-audit.json";
const BASE_SOURCE_SHA256: &str = "e6c6a610b4bab5dc9001b3846c087bdaba83fcd8c26908fd6eabc135f4ca2a9c";

fn main() {
    for path in [
        BASE_SOURCE_RELATIVE,
        BASE_FILE_MANIFEST_RELATIVE,
        BASE_CHECKSUMS_RELATIVE,
        BASE_RESOURCE_RELATIVE,
    ] {
        println!("cargo:rerun-if-changed={path}");
    }

    validate_base_manifest();

    for name in [
        "CARGO_FEATURE_SIGNED_REQUIRED",
        "CODEX_USB_GUARD_POLICY",
        "CODEX_USB_GUARD_PUBLIC_KEY_B64",
        "CODEX_USB_GUARD_PUBLIC_KEY_ID",
        "CODEX_USB_GUARD_KEYRING_JSON",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }

    let signed_required = env::var_os("CARGO_FEATURE_SIGNED_REQUIRED").is_some()
        || env::var("CODEX_USB_GUARD_POLICY").as_deref() == Ok("SignedRequired");
    let compiled_policy = if signed_required {
        "SignedRequired"
    } else {
        "DevelopmentAutoEnroll"
    };
    let out_dir = PathBuf::from(
        env::var_os("OUT_DIR").expect("Cargo 未提供 OUT_DIR，无法写入 USB 策略绑定标记"),
    );
    fs::write(
        out_dir.join("codex-usb-policy-marker.txt"),
        format!("codex-usb-compiled-policy:{compiled_policy}"),
    )
    .expect("无法写入 USB 策略绑定标记");
    if signed_required {
        validate_signed_required_config();
    }

    // Keep the build script sensitive to the USB build flag so Cargo rebuilds
    // when the release pipeline switches between the ordinary and USB
    // variants.  Tauri embeds its own RT_MANIFEST resource, so a linker
    // `/MANIFESTUAC` flag cannot override it (and `/MANIFEST:EMBED` would
    // create a duplicate resource).  The release pipeline patches only the
    // copied USB manager with the Windows SDK's mt.exe and verifies the final
    // PE resource.  The ordinary installed manager therefore remains
    // asInvoker, while the USB copy is explicitly requireAdministrator.
    println!("cargo:rerun-if-env-changed=CODEX_USB_MANAGER");
    tauri_build::build();
}

fn validate_base_manifest() {
    let manifest_dir = required_manifest_dir();
    let source_path = manifest_dir.join(BASE_SOURCE_RELATIVE);
    let file_manifest_path = manifest_dir.join(BASE_FILE_MANIFEST_RELATIVE);
    let checksums_path = manifest_dir.join(BASE_CHECKSUMS_RELATIVE);
    let resource_path = manifest_dir.join(BASE_RESOURCE_RELATIVE);

    let source_bytes = read_required_file(&source_path, "基础 Skill 源清单");
    let source_hash = sha256_hex(&canonical_text_bytes(&source_bytes));
    if source_hash != BASE_SOURCE_SHA256 {
        panic!(
            "H-03 基础 Skill 源清单 SHA-256 不匹配: expected {BASE_SOURCE_SHA256}, got {source_hash}"
        );
    }
    let source = parse_json(&source_bytes, "基础 Skill 源清单");
    let source_ids = validate_source_manifest(&source);

    let file_manifest_bytes = read_required_file(&file_manifest_path, "基础包 file-manifest");
    let file_manifest = parse_json(&file_manifest_bytes, "基础包 file-manifest");
    validate_file_manifest(&file_manifest, source_bytes.len() as u64, &source_hash);

    let checksums_bytes = read_required_file(&checksums_path, "基础包 checksums");
    validate_checksums(
        &checksums_bytes,
        &source_hash,
        &sha256_hex(&canonical_text_bytes(&file_manifest_bytes)),
    );

    let resource_bytes = read_required_file(&resource_path, "基础 Skill 静态资源");
    let resource = parse_json(&resource_bytes, "基础 Skill 静态资源");
    validate_resource(&resource, &source_ids, &resource_bytes);
}

fn required_manifest_dir() -> PathBuf {
    env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("H-03 缺少 CARGO_MANIFEST_DIR，无法定位基础包"))
}

fn read_required_file(path: &Path, label: &str) -> Vec<u8> {
    let metadata = fs::symlink_metadata(path)
        .unwrap_or_else(|error| panic!("H-03 {label} 缺失: {} ({error})", path.display()));
    if metadata.file_type().is_symlink() || metadata_is_reparse(&metadata) || !metadata.is_file() {
        panic!("H-03 {label} 不是安全的普通文件: {}", path.display());
    }
    fs::read(path)
        .unwrap_or_else(|error| panic!("H-03 读取 {label} 失败: {} ({error})", path.display()))
}

fn metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn parse_json(bytes: &[u8], label: &str) -> Value {
    serde_json::from_slice(bytes)
        .unwrap_or_else(|error| panic!("H-03 {label} JSON schema 无效: {error}"))
}

fn validate_source_manifest(value: &Value) -> Vec<String> {
    if value.get("schemaVersion").and_then(Value::as_u64) != Some(2) {
        panic!("H-03 基础 Skill 源清单必须是 schemaVersion=2");
    }
    let skills = value
        .get("skills")
        .and_then(Value::as_array)
        .filter(|skills| !skills.is_empty())
        .unwrap_or_else(|| panic!("H-03 基础 Skill 源清单缺少非空 skills 数组"));

    let mut ids = Vec::with_capacity(skills.len());
    let mut seen = HashSet::with_capacity(skills.len());
    for (index, skill) in skills.iter().enumerate() {
        let id = skill
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("H-03 基础 Skill 源清单 skills[{index}] 缺少字符串 id"));
        if !valid_base_id(id) {
            panic!("H-03 基础 Skill 源清单含非法 Skill ID: {id}");
        }
        if !seen.insert(id.to_ascii_lowercase()) {
            panic!("H-03 基础 Skill 源清单含重复 Skill ID: {id}");
        }
        ids.push(id.to_string());
    }
    ids
}

fn validate_file_manifest(value: &Value, source_bytes: u64, source_hash: &str) {
    if value.get("schemaVersion").and_then(Value::as_u64) != Some(2) {
        panic!("H-03 基础包 file-manifest 必须是 schemaVersion=2");
    }
    if value
        .get("hashScope")
        .and_then(Value::as_str)
        .map(|scope| scope.contains("text files use LF canonicalization"))
        != Some(true)
    {
        panic!("H-03 基础包 file-manifest 未声明 canonical-LF hash 规则");
    }
    let files = value
        .get("files")
        .and_then(Value::as_array)
        .filter(|files| !files.is_empty())
        .unwrap_or_else(|| panic!("H-03 基础包 file-manifest 缺少非空 files 数组"));

    let mut seen = HashSet::with_capacity(files.len());
    let mut source_entries = 0;
    for (index, entry) in files.iter().enumerate() {
        let path = entry
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("H-03 基础包 file-manifest files[{index}] 缺少 path"));
        if !seen.insert(path.to_ascii_lowercase()) {
            panic!("H-03 基础包 file-manifest 含重复路径: {path}");
        }
        let digest = entry
            .get("sha256")
            .and_then(Value::as_str)
            .unwrap_or_else(|| {
                panic!("H-03 基础包 file-manifest files[{index}] 缺少 sha256: {path}")
            });
        if !is_lower_hex_digest(digest) {
            panic!("H-03 基础包 file-manifest 含非法 SHA-256: {path}");
        }
        let bytes = entry
            .get("bytes")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| {
                panic!("H-03 基础包 file-manifest files[{index}] 缺少 bytes: {path}")
            });
        let owned = entry.get("owned").and_then(Value::as_bool);
        if path == BASE_SOURCE_PATH {
            source_entries += 1;
            if digest != source_hash || bytes != source_bytes || owned != Some(true) {
                panic!("H-03 基础包 file-manifest 的基础 Skill 源清单记录不匹配");
            }
        }
    }
    if source_entries != 1 {
        panic!("H-03 基础包 file-manifest 必须恰好包含一个基础 Skill 源清单记录");
    }
}

fn validate_checksums(bytes: &[u8], source_hash: &str, file_manifest_hash: &str) {
    let canonical = canonical_text_bytes(bytes);
    let text = std::str::from_utf8(&canonical)
        .unwrap_or_else(|error| panic!("H-03 基础包 checksums 不是 UTF-8: {error}"));
    let mut seen = HashSet::new();
    let mut source_entries = 0;
    let mut file_manifest_entries = 0;
    for (index, line) in text.split('\n').enumerate() {
        if line.is_empty() {
            continue;
        }
        if line.len() < 66 || &line[64..66] != " *" {
            panic!("H-03 基础包 checksums 第 {} 行格式无效", index + 1);
        }
        let digest = &line[..64];
        let path = &line[66..];
        if path.is_empty() || !is_lower_hex_digest(digest) || !seen.insert(path.to_string()) {
            panic!("H-03 基础包 checksums 第 {} 行无效或重复", index + 1);
        }
        match path {
            BASE_SOURCE_PATH => {
                source_entries += 1;
                if digest != source_hash {
                    panic!("H-03 基础包 checksums 的基础 Skill 源清单记录不匹配");
                }
            }
            "file-manifest.json" => {
                file_manifest_entries += 1;
                if digest != file_manifest_hash {
                    panic!("H-03 基础包 checksums 的 file-manifest 记录不匹配");
                }
            }
            _ => {}
        }
    }
    if source_entries != 1 || file_manifest_entries != 1 {
        panic!("H-03 基础包 checksums 缺少唯一的基础 Skill/file-manifest 记录");
    }
}

fn validate_resource(value: &Value, source_ids: &[String], resource_bytes: &[u8]) {
    if value.get("schemaVersion").and_then(Value::as_u64) != Some(1)
        || value.get("source").and_then(Value::as_str) != Some(BASE_RESOURCE_SOURCE)
        || value.get("sourceSha256").and_then(Value::as_str) != Some(BASE_SOURCE_SHA256)
    {
        panic!("H-03 基础 Skill 静态资源 schema/source/hash 不匹配");
    }
    let skills = value
        .get("skills")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("H-03 基础 Skill 静态资源缺少 skills 数组"));
    let mut resource_ids = Vec::with_capacity(skills.len());
    let mut seen = HashSet::with_capacity(skills.len());
    for (index, skill) in skills.iter().enumerate() {
        let id = skill
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("H-03 基础 Skill 静态资源 skills[{index}] 缺少字符串 id"));
        if !valid_base_id(id) || !seen.insert(id.to_ascii_lowercase()) {
            panic!("H-03 基础 Skill 静态资源含非法或重复 Skill ID: {id}");
        }
        resource_ids.push(id.to_string());
    }
    if resource_ids != source_ids {
        panic!("H-03 基础 Skill 静态资源 ID 集合/顺序与源清单不一致");
    }

    let expected = render_resource(source_ids);
    let resource_without_optional_lf = resource_bytes.strip_suffix(b"\n").unwrap_or(resource_bytes);
    if resource_without_optional_lf != expected.as_bytes() {
        panic!("H-03 基础 Skill 静态资源不是源清单的规范化副本");
    }
}

fn render_resource(ids: &[String]) -> String {
    let mut resource = format!(
        "{{\"schemaVersion\":1,\"source\":\"{BASE_RESOURCE_SOURCE}\",\"sourceSha256\":\"{BASE_SOURCE_SHA256}\",\"skills\":["
    );
    for (index, id) in ids.iter().enumerate() {
        if index > 0 {
            resource.push(',');
        }
        resource.push_str("{\"id\":\"");
        resource.push_str(id);
        resource.push_str("\"}");
    }
    resource.push_str("]}");
    resource
}

fn valid_base_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 64 {
        return false;
    }
    let mut chars = id.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_lowercase() || first.is_ascii_digit())
        || !chars.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '.' | '_' | '-')
        })
    {
        return false;
    }
    let stem = id.split('.').next().unwrap_or(id).to_ascii_uppercase();
    !matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) && !((stem.starts_with("COM") || stem.starts_with("LPT")) && stem[3..].parse::<u8>().is_ok())
}

fn is_lower_hex_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|character| character.is_ascii_digit() || matches!(character, b'a'..=b'f'))
}

fn canonical_text_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut canonical = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\r' {
            if bytes.get(index + 1) == Some(&b'\n') {
                index += 1;
            }
            canonical.push(b'\n');
        } else {
            canonical.push(bytes[index]);
        }
        index += 1;
    }
    canonical
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn validate_signed_required_config() {
    let public_key = required_env("CODEX_USB_GUARD_PUBLIC_KEY_B64");
    let decoded = decode_standard_base64(public_key.trim()).unwrap_or_else(|reason| {
        panic!("SignedRequired 构建的 CODEX_USB_GUARD_PUBLIC_KEY_B64 无效：{reason}")
    });
    if decoded.len() != 32 {
        panic!("SignedRequired 构建的 USB 公钥必须是 32 字节");
    }

    let key_id = required_env("CODEX_USB_GUARD_PUBLIC_KEY_ID");
    if !is_legal_key_id(&key_id) {
        panic!("SignedRequired 构建的 CODEX_USB_GUARD_PUBLIC_KEY_ID 不是合法 keyId");
    }
    validate_keyring_consistency(&key_id, &decoded);
}

fn validate_keyring_consistency(active_key_id: &str, active_public_key: &[u8]) {
    let Ok(raw_keyring) = env::var("CODEX_USB_GUARD_KEYRING_JSON") else {
        return;
    };
    let entries = serde_json::from_str::<Vec<Value>>(&raw_keyring)
        .unwrap_or_else(|error| panic!("SignedRequired 构建的 USB 公钥环 JSON 无效：{error}"));
    let mut active_entries = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let key_id = entry
            .get("keyId")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("USB 公钥环 entries[{index}] 缺少 keyId"));
        if !is_legal_key_id(key_id) {
            panic!("USB 公钥环 entries[{index}] 的 keyId 非法");
        }
        let public_key_b64 = entry
            .get("publicKeyB64")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("USB 公钥环 entries[{index}] 缺少 publicKeyB64"));
        let public_key = decode_standard_base64(public_key_b64.trim()).unwrap_or_else(|reason| {
            panic!("USB 公钥环 entries[{index}] 的 publicKeyB64 无效：{reason}")
        });
        if public_key.len() != 32 {
            panic!("USB 公钥环 entries[{index}] 的公钥必须是 32 字节");
        }
        if key_id == active_key_id {
            active_entries.push((index, public_key));
        }
    }
    if active_entries.len() != 1 {
        panic!(
            "USB 公钥环必须恰好包含一个 active keyId={active_key_id}，实际 {} 个",
            active_entries.len()
        );
    }
    let (index, keyring_public_key) = active_entries.pop().expect("checked exactly one entry");
    if keyring_public_key != active_public_key {
        panic!(
            "USB 公钥环 entries[{index}] 与 CODEX_USB_GUARD_PUBLIC_KEY_B64 不一致；拒绝生成可能无法验签的管理器"
        );
    }
}

fn required_env(name: &str) -> String {
    match env::var(name) {
        Ok(value) if !value.trim().is_empty() => value,
        _ => panic!("SignedRequired 构建必须配置 {name}"),
    }
}

fn is_legal_key_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
}

fn decode_standard_base64(value: &str) -> Result<Vec<u8>, &'static str> {
    let bytes = value.as_bytes();
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return Err("Base64 长度不是 4 的倍数");
    }

    let mut decoded = Vec::with_capacity(bytes.len() / 4 * 3);
    for (index, chunk) in bytes.chunks_exact(4).enumerate() {
        let is_last = index + 1 == bytes.len() / 4;
        let padding_two = chunk[2] == b'=';
        let padding_one = chunk[3] == b'=';
        if chunk[0] == b'='
            || chunk[1] == b'='
            || padding_two && !padding_one
            || padding_one && !is_last
        {
            return Err("Base64 填充位置无效");
        }

        let first = base64_value(chunk[0]).ok_or("Base64 字符无效")?;
        let second = base64_value(chunk[1]).ok_or("Base64 字符无效")?;
        let third = if padding_two {
            0
        } else {
            base64_value(chunk[2]).ok_or("Base64 字符无效")?
        };
        let fourth = if padding_one {
            0
        } else {
            base64_value(chunk[3]).ok_or("Base64 字符无效")?
        };

        if (padding_two && second & 0x0f != 0) || (padding_one && third & 0x03 != 0) {
            return Err("Base64 非规范编码");
        }

        decoded.push((first << 2) | (second >> 4));
        if !padding_two {
            decoded.push((second << 4) | (third >> 2));
        }
        if !padding_one {
            decoded.push((third << 6) | fourth);
        }
    }
    Ok(decoded)
}

fn base64_value(value: u8) -> Option<u8> {
    match value {
        b'A'..=b'Z' => Some(value - b'A'),
        b'a'..=b'z' => Some(value - b'a' + 26),
        b'0'..=b'9' => Some(value - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}
