//! Register the separately named USB-local Node REPL MCP service.
//!
//! The app-managed `node_repl` entry remains untouched. This entry exists so
//! the chat can still discover a Node REPL when the app-managed browser/CUA
//! feature path does not publish that service.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use toml::Value;

use crate::app::atomic_file;

const CONFIG_FILE: &str = "config.toml";
const SERVER_TABLE: &str = "[mcp_servers.node_repl_local]";
const SERVER_ENV_TABLE: &str = "[mcp_servers.node_repl_local.env]";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApplyOutcome {
    NotUsb,
    CodexRunning,
    MissingContext,
    Updated,
    AlreadyCurrent,
    ConfigMissing,
    ConfigUnreadable,
    UnsupportedConfigShape,
    WriteFailed,
}

fn quoted(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            ch => output.push(ch),
        }
    }
    output.push('"');
    output
}

fn registration_block(codex_home: &Path, bundle_root: &Path) -> String {
    let codex_resources = bundle_root.join("apps").join("Codex").join("resources");
    let node_root = codex_resources.join("cua_node").join("bin");
    let node = node_root.join("node.exe");
    let node_modules = node_root.join("node_modules");
    let node_repl = node_root.join("node_repl.exe");
    let codex_cli = codex_resources.join("codex.exe");
    let trusted_paths = format!("{};{}", codex_home.display(), node_modules.display());
    let trusted_services = r#"{"sky":"@oai/sky/service"}"#;

    format!(
        "{SERVER_TABLE}\n\
         enabled = true\n\
         command = {}\n\
         args = []\n\
         startup_timeout_sec = 120\n\n\
         {SERVER_ENV_TABLE}\n\
         CODEX_HOME = {}\n\
         CODEX_CLI_PATH = {}\n\
         NODE_REPL_NODE_PATH = {}\n\
         NODE_REPL_NODE_MODULE_DIRS = {}\n\
         NODE_REPL_TRUSTED_CODE_PATHS = {}\n\
         NODE_REPL_TRUSTED_SERVICES = {}\n\
         NODE_REPL_NATIVE_PIPE_CONNECT_TIMEOUT_MS = \"1000\"\n",
        quoted(&node_repl.to_string_lossy()),
        quoted(&codex_home.to_string_lossy()),
        quoted(&codex_cli.to_string_lossy()),
        quoted(&node.to_string_lossy()),
        quoted(&node_modules.to_string_lossy()),
        quoted(&trusted_paths),
        quoted(trusted_services),
    )
}

fn target_table_header(line: &str) -> bool {
    let trimmed = line.trim();
    let header = trimmed.split_once('#').map_or(trimmed, |(before, _)| before).trim();
    let inner = header
        .strip_prefix("[[")
        .and_then(|value| value.strip_suffix("]]"))
        .or_else(|| {
            header
                .strip_prefix('[')
                .and_then(|value| value.strip_suffix(']'))
        });
    let Some(inner) = inner else {
        return false;
    };
    let mut components = inner.split('.').map(str::trim);
    let Some(root) = components.next() else {
        return false;
    };
    let Some(server) = components.next() else {
        return false;
    };
    fn normalize_key(key: &str) -> &str {
        key.strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .or_else(|| key.strip_prefix('\'').and_then(|value| value.strip_suffix('\'')))
            .unwrap_or(key)
    }
    normalize_key(root) == "mcp_servers" && normalize_key(server) == "node_repl_local"
}

fn replace_registration(current: &str, block: &str) -> String {
    let mut retained = String::with_capacity(current.len() + block.len());
    let mut inside_managed_table = false;

    for line in current.split_inclusive('\n') {
        if line.trim_start().starts_with('[') {
            inside_managed_table = target_table_header(line);
        }
        // Comments can delimit the following capability-pack managed block.
        // They are not owned by the Node MCP table being replaced.
        if !inside_managed_table || line.trim_start().starts_with('#') {
            retained.push_str(line);
        }
    }

    if !retained.is_empty() && !retained.ends_with('\n') {
        retained.push('\n');
    }
    if !retained.is_empty() && !retained.ends_with("\n\n") {
        retained.push('\n');
    }
    retained.push_str(block);
    retained
}

fn plan_config(
    current: &str,
    codex_home: &Path,
    bundle_root: &Path,
) -> Result<String, &'static str> {
    let before: Value = current.parse().map_err(|_| "invalid-toml")?;
    if before
        .get("mcp_servers")
        .is_some_and(|servers| !servers.is_table())
    {
        return Err("unsupported-mcp-servers-shape");
    }
    let existing_local_table = before
        .get("mcp_servers")
        .and_then(Value::as_table)
        .and_then(|servers| servers.get("node_repl_local"));
    if existing_local_table.is_some_and(|value| !value.is_table()) {
        return Err("unsupported-node-repl-local-shape");
    }
    let has_table_block = current.lines().any(target_table_header);
    if existing_local_table.is_some() && !has_table_block {
        return Err("unsupported-node-repl-local-shape");
    }

    let mut block = registration_block(codex_home, bundle_root);
    if let Some(existing) = existing_local_table.and_then(Value::as_table) {
        let mut replacement: Value = block.parse().map_err(|_| "invalid-registration")?;
        let defaults = &replacement["mcp_servers"]["node_repl_local"];
        let mut server = existing.clone();
        let old_modules = existing
            .get("command")
            .and_then(Value::as_str)
            .and_then(|command| Path::new(command).parent())
            .map(|node_root| node_root.join("node_modules"));
        server.insert("command".into(), defaults["command"].clone());
        if let Some(env) = server.get_mut("env").and_then(Value::as_table_mut) {
            let old_home = env
                .get("CODEX_HOME")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let new_env = &defaults["env"];
            for key in ["NODE_REPL_NODE_MODULE_DIRS", "NODE_REPL_TRUSTED_CODE_PATHS"] {
                if let Some(value) = env.get_mut(key) {
                    if let Some(paths) = value.as_str() {
                        let updated = paths
                            .split(';')
                            .map(|entry| {
                                if old_modules.as_deref() == Some(Path::new(entry)) {
                                    new_env["NODE_REPL_NODE_MODULE_DIRS"]
                                        .as_str()
                                        .unwrap()
                                        .to_owned()
                                } else if key == "NODE_REPL_TRUSTED_CODE_PATHS"
                                    && old_home.as_deref().map(Path::new) == Some(Path::new(entry))
                                {
                                    codex_home.to_string_lossy().into_owned()
                                } else {
                                    entry.to_owned()
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(";");
                        *value = Value::String(updated);
                    }
                }
            }
            for key in ["CODEX_HOME", "CODEX_CLI_PATH", "NODE_REPL_NODE_PATH"] {
                if env.contains_key(key) {
                    env.insert(key.into(), new_env[key].clone());
                }
            }
        }
        if &server == existing {
            return Ok(current.to_owned());
        }
        replacement["mcp_servers"]["node_repl_local"] = Value::Table(server);
        block = toml::to_string(&replacement).map_err(|_| "invalid-registration")?;
    }
    let planned = replace_registration(current, &block);
    Ok(planned)
}

pub(crate) fn apply_for_launch(
    codex_home: &Path,
    bundle_root: &Path,
    usb_mode: bool,
    codex_running: bool,
) -> ApplyOutcome {
    if !usb_mode {
        return ApplyOutcome::NotUsb;
    }
    if codex_running {
        return ApplyOutcome::CodexRunning;
    }

    let path = codex_home.join(CONFIG_FILE);
    let original = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return ApplyOutcome::ConfigMissing,
        Err(_) => return ApplyOutcome::ConfigUnreadable,
    };
    let current = match std::str::from_utf8(&original) {
        Ok(text) => text,
        Err(_) => return ApplyOutcome::ConfigUnreadable,
    };
    let planned = match plan_config(current, codex_home, bundle_root) {
        Ok(planned) => planned,
        Err("invalid-toml" | "unsupported-mcp-servers-shape" | "unsupported-node-repl-local-shape") => {
            return ApplyOutcome::UnsupportedConfigShape;
        }
        Err(_) => return ApplyOutcome::ConfigUnreadable,
    };
    if planned == current {
        return ApplyOutcome::AlreadyCurrent;
    }
    if atomic_file::write_atomic_preserving_backup(&path, planned.as_bytes()).is_err() {
        return ApplyOutcome::WriteFailed;
    }
    ApplyOutcome::Updated
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"model = "test-model"
model_provider = "custom"

[model_providers.custom]
name = "USB provider"
base_url = "https://404token.xyz/v1"
wire_api = "responses"

[plugins."unified-computer-use@openai-bundled"]
enabled = true

[mcp_servers.node_repl]
command = "native-node-repl"
args = []
"#;

    fn paths() -> (std::path::PathBuf, std::path::PathBuf) {
        (
            Path::new("E:\\USB Test\\usb-codex-home").to_path_buf(),
            Path::new("E:\\USB Test").to_path_buf(),
        )
    }

    #[test]
    fn adds_one_local_entry_and_preserves_provider_plugin_and_native_entry() {
        let (home, root) = paths();
        let planned = plan_config(CONFIG, &home, &root).unwrap();
        let before: Value = CONFIG.parse().unwrap();
        let after: Value = planned.parse().unwrap();
        assert_eq!(before.get("model_provider"), after.get("model_provider"));
        assert_eq!(before.get("model_providers"), after.get("model_providers"));
        assert_eq!(before.get("plugins"), after.get("plugins"));
        assert_eq!(
            before
                .get("mcp_servers")
                .and_then(Value::as_table)
                .and_then(|servers| servers.get("node_repl")),
            after
                .get("mcp_servers")
                .and_then(Value::as_table)
                .and_then(|servers| servers.get("node_repl"))
        );
        assert_eq!(
            after
                .get("mcp_servers")
                .and_then(Value::as_table)
                .and_then(|servers| servers.get("node_repl_local"))
                .and_then(Value::as_table)
                .and_then(|server| server.get("enabled"))
                .and_then(Value::as_bool),
            Some(true)
        );
        assert!(planned.contains("node_repl.exe"));
        assert!(planned.contains("NODE_REPL_TRUSTED_SERVICES"));
        assert!(planned.contains("@oai/sky/service"));
    }

    #[test]
    fn refreshes_usb_paths_in_existing_local_entry_without_duplicate_tables() {
        let (home, root) = paths();
        let old = CONFIG.to_string()
            + "\n[mcp_servers.node_repl_local]\nenabled = true\ncommand = \"E:\\\\old\\\\node_repl.exe\"\nargs = []\nstartup_timeout_sec = 120\n\n[mcp_servers.node_repl_local.env]\nCODEX_HOME = \"E:\\\\old\\\\home\"\n";
        let planned = plan_config(&old, &home, &root).unwrap();
        let parsed: Value = planned.parse().unwrap();
        let server = parsed
            .get("mcp_servers")
            .and_then(Value::as_table)
            .and_then(|servers| servers.get("node_repl_local"))
            .and_then(Value::as_table)
            .unwrap();
        assert_eq!(server.get("command").and_then(Value::as_str).unwrap(), "E:\\USB Test\\apps\\Codex\\resources\\cua_node\\bin\\node_repl.exe");
        assert_eq!(planned.matches(SERVER_TABLE).count(), 1);
        assert_eq!(planned.matches(SERVER_ENV_TABLE).count(), 1);
        assert!(!planned.contains("E:\\\\old"));
        assert!(planned.contains("[mcp_servers.node_repl]"));
    }

    #[test]
    fn missing_config_is_not_created() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "codex-usb-node-repl-test-{}-{nonce}",
            std::process::id()
        ));
        let home = root.join("home");
        let bundle = root.join("bundle");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&bundle).unwrap();
        let config = home.join(CONFIG_FILE);
        assert_eq!(
            apply_for_launch(&home, &bundle, true, false),
            ApplyOutcome::ConfigMissing
        );
        assert!(!config.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn registration_update_is_idempotent() {
        let (home, root) = paths();
        let planned = plan_config(CONFIG, &home, &root).unwrap();
        assert_eq!(plan_config(&planned, &home, &root).unwrap(), planned);
    }

    #[test]
    fn package_path_update_preserves_following_capability_config_markers() {
        let old_block = registration_block(Path::new(r"E:\old\home"), Path::new(r"E:\old"));
        let capability = concat!(
            "# BEGIN CODEX-STARTER-KIT MANAGED CONFIG v1\n",
            "# Generated by codex-starter-kit.\n",
            "[[skills.config]]\npath = 'fixture/SKILL.md'\nenabled = true\n",
            "[mcp_servers.cloudflare]\nenabled = false\nurl = 'https://example.invalid/mcp'\n",
            "# END CODEX-STARTER-KIT MANAGED CONFIG v1\n",
        );
        let current = format!("{CONFIG}{old_block}\n{capability}");
        let before: Value = current.parse().unwrap();
        let (home, root) = paths();
        let planned = plan_config(&current, &home, &root).unwrap();
        assert!(planned.contains(capability));
        let after: Value = planned.parse().unwrap();
        assert_eq!(after["skills"], before["skills"]);
        assert_eq!(after["mcp_servers"]["cloudflare"], before["mcp_servers"]["cloudflare"]);
        assert_eq!(plan_config(&planned, &home, &root).unwrap(), planned);
    }

    #[test]
    fn migration_preserves_options_environment_and_extra_directories() {
        let old_home = Path::new(r"E:\old\home");
        let old_root = Path::new(r"E:\old");
        let old_block = registration_block(old_home, old_root);
        let mut original: Value = (CONFIG.to_owned() + &old_block).parse().unwrap();
        let server = original["mcp_servers"]["node_repl_local"]
            .as_table_mut()
            .unwrap();
        server.insert("enabled".into(), Value::Boolean(false));
        server.insert(
            "args".into(),
            Value::Array(vec![Value::String("--user-option".into())]),
        );
        server.insert("startup_timeout_sec".into(), Value::Integer(45));
        server.insert("tool_timeout_sec".into(), Value::Integer(90));
        let env = server.get_mut("env").unwrap().as_table_mut().unwrap();
        for key in ["NODE_REPL_NODE_MODULE_DIRS", "NODE_REPL_TRUSTED_CODE_PATHS"] {
            let paths = format!(r"{};D:\user modules", env[key].as_str().unwrap());
            env.insert(key.into(), Value::String(paths));
        }
        env.insert("USER_SETTING".into(), Value::String("keep".into()));
        env.insert(
            "NODE_REPL_NATIVE_PIPE_CONNECT_TIMEOUT_MS".into(),
            Value::String("2500".into()),
        );
        env.insert(
            "NODE_REPL_TRUSTED_SERVICES".into(),
            Value::String(r#"{"sky":"@oai/sky/service","custom":"custom/service"}"#.into()),
        );
        let current = toml::to_string(&original).unwrap();
        let (home, root) = paths();
        let planned = plan_config(&current, &home, &root).unwrap();
        let after: Value = planned.parse().unwrap();
        let defaults: Value = registration_block(&home, &root).parse().unwrap();
        let before_server = &original["mcp_servers"]["node_repl_local"];
        let after_server = &after["mcp_servers"]["node_repl_local"];
        let default_server = &defaults["mcp_servers"]["node_repl_local"];
        for key in ["enabled", "args", "startup_timeout_sec", "tool_timeout_sec"] {
            assert_eq!(after_server[key], before_server[key], "{key}");
        }
        assert_eq!(after_server["command"], default_server["command"]);
        for key in ["CODEX_HOME", "CODEX_CLI_PATH", "NODE_REPL_NODE_PATH"] {
            assert_eq!(
                after_server["env"][key], default_server["env"][key],
                "{key}"
            );
        }
        for key in ["NODE_REPL_NODE_MODULE_DIRS", "NODE_REPL_TRUSTED_CODE_PATHS"] {
            assert_eq!(
                after_server["env"][key].as_str().unwrap(),
                format!(
                    r"{};D:\user modules",
                    default_server["env"][key].as_str().unwrap()
                )
            );
        }
        for key in [
            "USER_SETTING",
            "NODE_REPL_NATIVE_PIPE_CONNECT_TIMEOUT_MS",
            "NODE_REPL_TRUSTED_SERVICES",
        ] {
            assert_eq!(after_server["env"][key], before_server["env"][key], "{key}");
        }
        assert_eq!(after["model_providers"], original["model_providers"]);
        assert_eq!(after["plugins"], original["plugins"]);
        assert_eq!(
            after["mcp_servers"]["node_repl"],
            original["mcp_servers"]["node_repl"]
        );
        assert_eq!(plan_config(&planned, &home, &root).unwrap(), planned);
    }

    #[test]
    fn unchanged_paths_preserve_text_and_do_not_restore_removed_options() {
        let (home, root) = paths();
        let current = (CONFIG.to_owned() + &registration_block(&home, &root))
            .replace(
                "enabled = true\ncommand",
                "# Keep disabled\nenabled = false\ncommand",
            )
            .replace("args = []\nstartup_timeout_sec = 120\n", "");
        assert_eq!(plan_config(&current, &home, &root).unwrap(), current);
    }

    #[test]
    fn non_usb_or_running_codex_skips_registration() {
        assert_eq!(
            apply_for_launch(Path::new("unused"), Path::new("unused"), false, false),
            ApplyOutcome::NotUsb
        );
        assert_eq!(
            apply_for_launch(Path::new("unused"), Path::new("unused"), true, true),
            ApplyOutcome::CodexRunning
        );
    }

    #[test]
    fn unsupported_inline_registration_is_not_duplicated() {
        let (home, root) = paths();
        let inline = "model = \"test\"\nmcp_servers.node_repl_local = { enabled = true }\n";
        assert_eq!(
            plan_config(inline, &home, &root),
            Err("unsupported-node-repl-local-shape")
        );
    }
}
