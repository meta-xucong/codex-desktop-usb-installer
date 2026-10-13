use std::collections::HashSet;
use std::env;
use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use ed25519_dalek::VerifyingKey;
use serde_json::Value;
use sha2::{Digest, Sha256};

const BASE_SOURCE_RELATIVE: &str = "../installer-pack/manifest/skill-audit.json";
const BASE_FILE_MANIFEST_RELATIVE: &str = "../installer-pack/file-manifest.json";
const BASE_CHECKSUMS_RELATIVE: &str = "../installer-pack/checksums.sha256";
const BASE_RESOURCE_RELATIVE: &str = "resources/skill_repo_base_manifest.json";
const SKILL_HUB_SOURCES_RELATIVE: &str = "resources/skill_hub_sources.json";
const BASE_SOURCE_PATH: &str = "manifest/skill-audit.json";
const BASE_RESOURCE_SOURCE: &str = "installer-pack/manifest/skill-audit.json";
const BASE_SOURCE_SHA256: &str = "67a3bb12666830575cb406bdb59c1f19902544df53737747bf02d588b90ded0d";

// Release pins bind signed USB managers to the production license issuer and
// the VPS that signs revocation decisions. Rotate the status public key only
// with a coordinated server keyring and client release.
const RELEASE_USB_REVOCATION_HOST: &str = "license.404token.xyz";
const RELEASE_USB_LICENSE_KEY_ID: &str = "prod-2026-10-03";
const RELEASE_USB_LICENSE_KEY_SHA256: &str =
    "0f637ced3c7bf1cb97072b522bd1f156fe703bdc8e711a59c68698487f065aa5";
const RELEASE_REVOCATION_KEY_PINS: &[(&str, &str)] = &[(
    "revocation-20261010",
    "b6eec73a151000067a41da78cd523d1e67536a3a601f6fc219f437b443016182",
)];

fn main() {
    for path in [
        BASE_SOURCE_RELATIVE,
        BASE_FILE_MANIFEST_RELATIVE,
        BASE_CHECKSUMS_RELATIVE,
        BASE_RESOURCE_RELATIVE,
        SKILL_HUB_SOURCES_RELATIVE,
    ] {
        println!("cargo:rerun-if-changed={path}");
    }

    validate_base_manifest();
    validate_skill_hub_sources();

    for name in [
        "CARGO_FEATURE_SIGNED_REQUIRED",
        "CODEX_USB_GUARD_POLICY",
        "CODEX_USB_GUARD_PUBLIC_KEY_B64",
        "CODEX_USB_GUARD_PUBLIC_KEY_ID",
        "CODEX_USB_GUARD_KEYRING_JSON",
        "CODEX_USB_REVOCATION_URL",
        "CODEX_USB_REVOCATION_KEYRING_JSON",
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

fn validate_skill_hub_sources() {
    let manifest_dir = required_manifest_dir();
    let path = manifest_dir.join(SKILL_HUB_SOURCES_RELATIVE);
    let bytes = read_required_file(&path, "Skill Hub 来源目录");
    let value = parse_json(&bytes, "Skill Hub 来源目录");
    if value.get("schemaVersion").and_then(Value::as_u64) != Some(1)
        || value.get("channel").and_then(Value::as_str) != Some("skill-hub")
    {
        panic!("H-SKILL-01 Skill Hub 来源目录 schema/channel 无效");
    }
    let sources = value
        .get("sources")
        .and_then(Value::as_array)
        .filter(|sources| !sources.is_empty())
        .unwrap_or_else(|| panic!("H-SKILL-01 Skill Hub 来源目录缺少 sources"));

    let mut ids = HashSet::new();
    let mut priorities = HashSet::new();
    let mut bases = HashSet::new();
    let mut relay_count = 0usize;
    let mut github_count = 0usize;

    for (index, source) in sources.iter().enumerate() {
        let id = source
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("H-SKILL-01 sources[{index}] 缺少 id"));
        let kind = source
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("H-SKILL-01 sources[{index}] 缺少 kind"));
        let priority = source
            .get("priority")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| panic!("H-SKILL-01 sources[{index}] 缺少 priority"));
        let enabled = source
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or_else(|| panic!("H-SKILL-01 sources[{index}] 缺少 enabled"));
        let test_fixture = source
            .get("testFixture")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        if id.is_empty()
            || id.len() > 64
            || !id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            || !ids.insert(id.to_string())
            || priority == 0
            || !priorities.insert(priority)
        {
            panic!("H-SKILL-01 sources[{index}] 的 id/priority 无效或重复");
        }

        match kind {
            "domestic" => {
                let suffix = id
                    .strip_prefix("cn-mirror-")
                    .filter(|suffix| {
                        !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit())
                    })
                    .unwrap_or_else(|| panic!("H-SKILL-01 国内来源 ID 无效: {id}"));
                let _ = suffix;
                let base = source
                    .get("base")
                    .and_then(Value::as_str)
                    .unwrap_or_else(|| panic!("H-SKILL-01 国内来源缺少 base: {id}"));
                validate_skill_hub_base(base, true, test_fixture, id);
                if enabled && test_fixture {
                    panic!("H-SKILL-01 testFixture 不能启用为生产来源: {id}");
                }
                if !bases.insert(base.to_string()) {
                    panic!("H-SKILL-01 静态来源 base 重复: {id}");
                }
            }
            "relay" => {
                relay_count += 1;
                if id != "ph-relay"
                    || !enabled
                    || test_fixture
                    || source.get("base").and_then(Value::as_str)
                        != Some("https://update.404token.xyz/__codex_update__/skill-hub/")
                {
                    panic!("H-SKILL-01 Relay 来源身份无效");
                }
                let base = source
                    .get("base")
                    .and_then(Value::as_str)
                    .expect("H-SKILL-01 Relay 缺少 base");
                validate_skill_hub_base(base, true, false, id);
                if !bases.insert(base.to_string()) {
                    panic!("H-SKILL-01 静态来源 base 重复: {id}");
                }
            }
            "github" => {
                github_count += 1;
                if id != "github" || !enabled || test_fixture || source.get("base").is_some() {
                    panic!("H-SKILL-01 GitHub 来源身份无效");
                }
            }
            _ => panic!("H-SKILL-01 来源 kind 不支持: {kind}"),
        }
    }

    if relay_count != 1 || github_count != 1 {
        panic!("H-SKILL-01 来源目录必须有一个合法启用的 Relay 和一个合法启用的 GitHub");
    }
}

fn validate_skill_hub_base(base: &str, require_skill_hub_path: bool, test_fixture: bool, id: &str) {
    if base.len() > 512
        || !base.starts_with("https://")
        || !base.ends_with("/skill-hub/")
        || base.contains(['?', '#', '@'])
    {
        panic!("H-SKILL-01 来源 base 边界无效: {id}");
    }
    let authority_and_path = &base["https://".len()..];
    let (authority, path) = authority_and_path
        .split_once('/')
        .unwrap_or_else(|| panic!("H-SKILL-01 来源 base 缺少路径: {id}"));
    if require_skill_hub_path && !path.ends_with("skill-hub/") {
        panic!("H-SKILL-01 来源 base 路径无效: {id}");
    }
    if authority.is_empty()
        || authority.contains(':')
        || authority.contains("..")
        || !authority.contains('.')
        || authority.parse::<IpAddr>().is_ok()
    {
        panic!("H-SKILL-01 来源必须使用 HTTPS 公网域名: {id}");
    }
    let reserved = authority == "example"
        || authority.ends_with(".example")
        || authority.ends_with(".invalid")
        || authority.ends_with(".test");
    if reserved != test_fixture {
        panic!("H-SKILL-01 保留域名必须显式标记 testFixture: {id}");
    }
    if test_fixture && !id.starts_with("cn-mirror-") {
        panic!("H-SKILL-01 testFixture 只能用于国内来源: {id}");
    }
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
    validate_revocation_config(&key_id, &decoded);
}

fn validate_revocation_config(license_key_id: &str, license_public_key: &[u8]) {
    let raw_url = required_env("CODEX_USB_REVOCATION_URL");
    let url = url::Url::parse(&raw_url)
        .unwrap_or_else(|error| panic!("SignedRequired 构建的授权服务 URL 无效：{error}"));
    if url.scheme() != "https"
        || url.host_str().is_none()
        || url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || raw_url.contains("<")
        || raw_url.contains(">")
    {
        panic!("SignedRequired 构建的授权服务 URL 必须是无凭据的正式 HTTPS URL");
    }

    let raw_keyring = required_env("CODEX_USB_REVOCATION_KEYRING_JSON");
    let keyring = serde_json::from_str::<Value>(&raw_keyring)
        .unwrap_or_else(|error| panic!("SignedRequired 构建的授权状态验签公钥环无效：{error}"));
    let keyring_object = keyring
        .as_object()
        .filter(|object| object.len() == 1 && object.contains_key("keys"))
        .unwrap_or_else(|| panic!("授权状态公钥环 JSON 只能包含 keys 字段"));
    let keys = keyring_object
        .get("keys")
        .and_then(Value::as_object)
        .filter(|keys| !keys.is_empty())
        .unwrap_or_else(|| panic!("授权状态公钥环必须包含非空 keys 对象"));
    let mut parsed_keys = Vec::with_capacity(keys.len());
    for (key_id, encoded) in keys {
        if !is_legal_key_id(key_id) {
            panic!("授权状态公钥环包含非法 signingKeyId: {key_id}");
        }
        let encoded = encoded
            .as_str()
            .unwrap_or_else(|| panic!("授权状态公钥 {key_id} 必须是 Base64 字符串"));
        let decoded = decode_standard_base64(encoded.trim())
            .unwrap_or_else(|reason| panic!("授权状态公钥 {key_id} Base64 无效：{reason}"));
        if decoded.len() != 32 {
            panic!("授权状态公钥 {key_id} 必须是 32 字节 Ed25519 公钥");
        }
        let public_key: [u8; 32] = decoded
            .as_slice()
            .try_into()
            .unwrap_or_else(|_| panic!("授权状态公钥 {key_id} 必须是 32 字节 Ed25519 公钥"));
        VerifyingKey::from_bytes(&public_key).unwrap_or_else(|error| {
            panic!("授权状态公钥 {key_id} 不是可解析的 Ed25519 公钥：{error}")
        });
        parsed_keys.push((key_id.as_str(), decoded));
    }

    if !release_revocation_pins_configured() {
        panic!(
            "SignedRequired 构建已拒绝：USB 授权服务 host、USB license 公钥 pin、revocation 公钥 allowlist 尚未配置生产值"
        );
    }
    if url.host_str() != Some(RELEASE_USB_REVOCATION_HOST)
        || url.port().is_some_and(|port| port != 443)
    {
        panic!(
            "SignedRequired 构建的授权服务 URL 必须使用固定的生产 HTTPS host https://{}/",
            RELEASE_USB_REVOCATION_HOST
        );
    }

    let license_key_fingerprint = sha256_hex(license_public_key);
    if license_key_id != RELEASE_USB_LICENSE_KEY_ID
        || license_key_fingerprint != RELEASE_USB_LICENSE_KEY_SHA256
    {
        panic!(
            "SignedRequired 构建的 USB license 公钥与固定生产 pin 不匹配 (keyId={license_key_id}, sha256={license_key_fingerprint})"
        );
    }

    if parsed_keys.len() != RELEASE_REVOCATION_KEY_PINS.len() {
        panic!("SignedRequired 构建的授权状态公钥环条目数与固定生产 allowlist 不一致");
    }
    for (key_id, decoded) in parsed_keys {
        let expected_fingerprint = RELEASE_REVOCATION_KEY_PINS
            .iter()
            .find_map(|(pinned_key_id, fingerprint)| {
                (*pinned_key_id == key_id).then_some(*fingerprint)
            })
            .unwrap_or_else(|| panic!("授权状态公钥环包含未获批准的生产 signingKeyId: {key_id}"));
        let fingerprint = sha256_hex(&decoded);
        if fingerprint != expected_fingerprint {
            panic!("授权状态公钥 {key_id} 与固定生产 SHA-256 pin 不匹配 (sha256={fingerprint})");
        }
    }
}

fn release_revocation_pins_configured() -> bool {
    !RELEASE_USB_REVOCATION_HOST.is_empty()
        && !RELEASE_USB_LICENSE_KEY_ID.is_empty()
        && is_lower_hex_digest(RELEASE_USB_LICENSE_KEY_SHA256)
        && !RELEASE_REVOCATION_KEY_PINS.is_empty()
        && RELEASE_REVOCATION_KEY_PINS
            .iter()
            .all(|(key_id, fingerprint)| {
                is_legal_key_id(key_id) && is_lower_hex_digest(fingerprint)
            })
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
    for (index, chunk) in bytes.as_chunks::<4>().0.iter().enumerate() {
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
