//! Keep the USB 404token profile from selecting the prose-only Work Browser
//! preflight when the local Codex instance has no ChatGPT principal.
//!
//! This changes only `[desktop].conversationDetailMode`, only for the active
//! provider configured with the product's canonical 404token endpoint, and
//! when its local preference is `STEPS_PROSE` or missing. A missing preference
//! is made explicit so the app's legacy global-state migration cannot restore
//! an older prose value during startup.
//! The Codex app's execution assignment policy can still override this local
//! preference; this module does not alter or infer that policy.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use toml::Value;

use crate::app::{atomic_file, codex_config_readback};

const CONFIG_FILE: &str = "config.toml";
const PROSE_MODE: &str = "STEPS_PROSE";
const COMMANDS_MODE: &str = "STEPS_COMMANDS";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigMode {
    Missing,
    Prose,
    Commands,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApplyOutcome {
    NotUsb,
    CodexRunning,
    MissingConfig,
    OtherProvider,
    AlreadyCompatible,
    Updated,
    ConfigUnreadable,
    UnsupportedConfigShape,
    WriteFailed,
}

fn detail_mode(document: &Value) -> ConfigMode {
    let Some(desktop) = document.get("desktop") else {
        return ConfigMode::Missing;
    };
    let Some(desktop) = desktop.as_table() else {
        return ConfigMode::Other;
    };
    match desktop.get("conversationDetailMode") {
        None => ConfigMode::Missing,
        Some(Value::String(mode)) if mode == PROSE_MODE => ConfigMode::Prose,
        Some(Value::String(mode)) if mode == COMMANDS_MODE => ConfigMode::Commands,
        Some(_) => ConfigMode::Other,
    }
}

#[cfg(test)]
fn without_managed_mode(mut document: Value) -> Value {
    let remove_empty_desktop =
        if let Some(desktop) = document.get_mut("desktop").and_then(Value::as_table_mut) {
            desktop.remove("conversationDetailMode");
            desktop.is_empty()
        } else {
            false
        };
    if remove_empty_desktop {
        document.as_table_mut().unwrap().remove("desktop");
    }
    document
}

fn header_name(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let inner = trimmed.strip_prefix('[')?.strip_suffix(']')?.trim();
    if inner.starts_with('[') {
        return None;
    }
    if inner.len() >= 2 {
        let bytes = inner.as_bytes();
        if (bytes[0] == b'"' && bytes[inner.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[inner.len() - 1] == b'\'')
        {
            return Some(&inner[1..inner.len() - 1]);
        }
    }
    Some(inner)
}

fn mode_key_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return false;
    }
    trimmed
        .split_once('=')
        .map(|(key, _)| key.trim() == "conversationDetailMode")
        .unwrap_or(false)
}

fn comment_start(line: &str) -> Option<usize> {
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        if let Some(delimiter) = quote {
            if delimiter == '"' && escaped {
                escaped = false;
            } else if delimiter == '"' && ch == '\\' {
                escaped = true;
            } else if ch == delimiter {
                quote = None;
            }
        } else if ch == '"' || ch == '\'' {
            quote = Some(ch);
        } else if ch == '#' {
            return Some(index);
        }
    }
    None
}

fn replace_mode_line(line: &str) -> Option<String> {
    let (content, ending) = if let Some(content) = line.strip_suffix("\r\n") {
        (content, "\r\n")
    } else if let Some(content) = line.strip_suffix('\n') {
        (content, "\n")
    } else {
        (line, "")
    };
    let end = comment_start(content).unwrap_or(content.len());
    let (assignment, comment) = content.split_at(end);
    let equals = assignment.find('=')?;
    let value_area = &assignment[equals + 1..];
    let leading = &value_area[..value_area.len() - value_area.trim_start().len()];
    let trailing = &value_area[value_area.trim_end().len()..];
    let mut updated = String::with_capacity(line.len() + 8);
    updated.push_str(&assignment[..=equals]);
    updated.push_str(leading);
    updated.push('"');
    updated.push_str(COMMANDS_MODE);
    updated.push('"');
    updated.push_str(trailing);
    updated.push_str(comment);
    updated.push_str(ending);
    Some(updated)
}

fn replace_desktop_mode_line(current: &str) -> Result<String, &'static str> {
    let lines: Vec<&str> = current.split_inclusive('\n').collect();
    let mut desktop_sections = Vec::new();
    let mut active_start = None;
    for (index, line) in lines.iter().enumerate() {
        if let Some(name) = header_name(line) {
            if let Some(start) = active_start.take() {
                desktop_sections.push((start, index));
            }
            if name == "desktop" {
                active_start = Some(index);
            }
        }
    }
    if let Some(start) = active_start {
        desktop_sections.push((start, lines.len()));
    }
    if desktop_sections.len() != 1 {
        return Err("unsupported-desktop-table");
    }
    let (start, end) = desktop_sections[0];
    let mode_lines: Vec<usize> = ((start + 1)..end)
        .filter(|&index| mode_key_line(lines[index]))
        .collect();
    if mode_lines.len() != 1 {
        return Err("unsupported-detail-mode");
    }
    let replacement = replace_mode_line(lines[mode_lines[0]]).ok_or("unsupported-detail-mode")?;
    let mut planned = String::with_capacity(current.len() + 8);
    for (index, line) in lines.iter().enumerate() {
        if index == mode_lines[0] {
            planned.push_str(&replacement);
        } else {
            planned.push_str(line);
        }
    }
    Ok(planned)
}

fn newline_style(current: &str) -> &'static str {
    if current.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn insert_missing_desktop_mode(current: &str, document: &Value) -> Result<String, &'static str> {
    let lines: Vec<&str> = current.split_inclusive('\n').collect();
    let mut desktop_sections = Vec::new();
    let mut active_start = None;
    let mut first_section_start = None;
    for (index, line) in lines.iter().enumerate() {
        if let Some(name) = header_name(line) {
            first_section_start.get_or_insert(index);
            if let Some(start) = active_start.take() {
                desktop_sections.push((start, index));
            }
            if name == "desktop" {
                active_start = Some(index);
            }
        }
    }
    if let Some(start) = active_start {
        desktop_sections.push((start, lines.len()));
    }
    if desktop_sections.len() > 1 {
        return Err("unsupported-desktop-table");
    }

    let newline = newline_style(current);
    let setting = format!("conversationDetailMode = \"{COMMANDS_MODE}\"{newline}");
    if let Some((start, _)) = desktop_sections.first().copied() {
        let insert_at = lines[..=start].iter().map(|line| line.len()).sum::<usize>();
        let mut planned = String::with_capacity(current.len() + setting.len());
        planned.push_str(&current[..insert_at]);
        if insert_at > 0 && !planned.ends_with('\n') {
            planned.push_str(newline);
        }
        planned.push_str(&setting);
        planned.push_str(&current[insert_at..]);
        return Ok(planned);
    }

    if document.get("desktop").is_some() {
        // A table created by dotted keys or a child table has no explicit
        // [desktop] header. Add its missing key at the root before tables;
        // inline tables cannot be extended this way without rewriting them.
        let root_end = first_section_start
            .map(|index| lines[..index].iter().map(|line| line.len()).sum::<usize>())
            .unwrap_or(current.len());
        let root = &current[..root_end];
        if root.lines().any(|line| {
            let Some((key, value)) = line.split_once('=') else {
                return false;
            };
            key.trim() == "desktop" && value.trim_start().starts_with('{')
        }) {
            return Err("unsupported-desktop-table");
        }

        let mut planned = String::with_capacity(current.len() + setting.len() + 32);
        planned.push_str(root);
        if !root.is_empty() && !root.ends_with('\n') {
            planned.push_str(newline);
        }
        planned.push_str(&format!("desktop.{setting}"));
        planned.push_str(&current[root_end..]);
        return Ok(planned);
    }

    let mut planned = String::with_capacity(current.len() + setting.len() + 16);
    planned.push_str(current);
    if !current.is_empty() {
        if !planned.ends_with('\n') {
            planned.push_str(newline);
        }
        if !planned.ends_with(&format!("{newline}{newline}")) {
            planned.push_str(newline);
        }
    }
    planned.push_str("[desktop]");
    planned.push_str(newline);
    planned.push_str(&setting);
    Ok(planned)
}

/// Pure, format-preserving config plan. The TOML AST identifies the active
/// provider and mode; only the matching assignment line is replaced.
fn plan_config(current: &str, usb_mode: bool) -> Result<Option<String>, &'static str> {
    if !usb_mode {
        return Ok(None);
    }

    let before: Value = current.parse().map_err(|_| "invalid-toml")?;
    if !codex_config_readback::active_provider_uses_404token_endpoint(&before) {
        return Ok(None);
    }
    match detail_mode(&before) {
        ConfigMode::Prose => Ok(Some(replace_desktop_mode_line(current)?)),
        ConfigMode::Missing => Ok(Some(insert_missing_desktop_mode(current, &before)?)),
        ConfigMode::Commands | ConfigMode::Other => Ok(None),
    }
}

/// Apply the one-key adjustment immediately before a USB Codex launch. Running
/// Codex instances are left untouched to avoid racing their config writes.
pub(crate) fn apply_for_launch(
    codex_home: &Path,
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
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return ApplyOutcome::MissingConfig;
        }
        Err(_) => return ApplyOutcome::ConfigUnreadable,
    };
    let current = match std::str::from_utf8(&original) {
        Ok(text) => text,
        Err(_) => return ApplyOutcome::ConfigUnreadable,
    };
    let planned = match plan_config(current, true) {
        Ok(Some(planned)) => planned,
        Ok(None) => {
            let parsed: Value = match current.parse() {
                Ok(parsed) => parsed,
                Err(_) => return ApplyOutcome::ConfigUnreadable,
            };
            if !codex_config_readback::active_provider_uses_404token_endpoint(&parsed) {
                return ApplyOutcome::OtherProvider;
            }
            return ApplyOutcome::AlreadyCompatible;
        }
        Err("unsupported-desktop-table" | "unsupported-detail-mode") => {
            return ApplyOutcome::UnsupportedConfigShape;
        }
        Err(_) => return ApplyOutcome::ConfigUnreadable,
    };

    if atomic_file::write_atomic_preserving_backup(&path, planned.as_bytes()).is_err() {
        return ApplyOutcome::WriteFailed;
    }
    ApplyOutcome::Updated
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG_WITH_PROSE: &str = r#"model = "test-model"
model_provider = "404token"

[model_providers.404token]
name = "USB provider"
base_url = "https://404token.xyz/v1"
wire_api = "responses"
requires_openai_auth = true

[desktop]
conversationDetailMode = "STEPS_PROSE" # preserve this comment
browserEnabled = true

[mcp_servers.node_repl]
command = "node"
args = ["server.js"]
"#;

    #[test]
    fn usb_404token_prose_changes_only_the_detail_mode() {
        let planned = plan_config(CONFIG_WITH_PROSE, true)
            .unwrap()
            .expect("the selected USB provider should be adjusted");
        let before: Value = CONFIG_WITH_PROSE.parse().unwrap();
        let after: Value = planned.parse().unwrap();
        assert_eq!(detail_mode(&after), ConfigMode::Commands);
        assert_eq!(without_managed_mode(before), without_managed_mode(after));
        assert!(
            planned.contains("conversationDetailMode = \"STEPS_COMMANDS\" # preserve this comment")
        );
        assert!(planned.contains("[mcp_servers.node_repl]"));
        assert!(planned.contains("[model_providers.404token]"));
        assert!(planned.contains("browserEnabled = true"));
    }

    #[test]
    fn non_usb_launch_keeps_config_unchanged() {
        assert_eq!(plan_config(CONFIG_WITH_PROSE, false).unwrap(), None);
    }

    #[test]
    fn other_provider_keeps_config_unchanged() {
        let config = CONFIG_WITH_PROSE.replace(
            "model_provider = \"404token\"",
            "model_provider = \"openai\"",
        );
        assert_eq!(plan_config(&config, true).unwrap(), None);
    }

    #[test]
    fn custom_provider_is_scoped_by_404token_endpoint() {
        let custom_404token = CONFIG_WITH_PROSE
            .replace(
                "model_provider = \"404token\"",
                "model_provider = \"custom\"",
            )
            .replace("[model_providers.404token]", "[model_providers.custom]");
        let planned = plan_config(&custom_404token, true).unwrap().unwrap();
        let after: Value = planned.parse().unwrap();
        assert_eq!(detail_mode(&after), ConfigMode::Commands);
        assert_eq!(
            codex_config_readback::active_model_provider(&after),
            Some("custom")
        );

        let custom_other =
            custom_404token.replace("https://404token.xyz/v1", "https://other.example/v1");
        assert_eq!(plan_config(&custom_other, true).unwrap(), None);
    }

    #[test]
    fn commands_and_unknown_mode_are_left_unchanged() {
        let commands = CONFIG_WITH_PROSE.replace(PROSE_MODE, COMMANDS_MODE);
        assert_eq!(plan_config(&commands, true).unwrap(), None);
        let unknown = CONFIG_WITH_PROSE.replace(PROSE_MODE, "STEPS_USER_DEFINED");
        assert_eq!(plan_config(&unknown, true).unwrap(), None);
    }

    #[test]
    fn missing_mode_is_written_in_existing_desktop_table() {
        let missing = CONFIG_WITH_PROSE.replace(
            "conversationDetailMode = \"STEPS_PROSE\" # preserve this comment\n",
            "",
        );
        let planned = plan_config(&missing, true).unwrap().unwrap();
        let before: Value = missing.parse().unwrap();
        let after: Value = planned.parse().unwrap();
        assert_eq!(detail_mode(&after), ConfigMode::Commands);
        assert_eq!(without_managed_mode(before), without_managed_mode(after));
        assert!(planned.contains(
            "[desktop]\nconversationDetailMode = \"STEPS_COMMANDS\"\nbrowserEnabled = true"
        ));
        assert!(planned.contains("[mcp_servers.node_repl]"));
    }

    #[test]
    fn missing_desktop_table_is_appended_without_changing_existing_config() {
        let config = r#"model_provider = "custom"

[model_providers.custom]
base_url = "https://404token.xyz/v1"
"#;
        let planned = plan_config(config, true).unwrap().unwrap();
        let before: Value = config.parse().unwrap();
        let after: Value = planned.parse().unwrap();
        assert_eq!(detail_mode(&after), ConfigMode::Commands);
        assert_eq!(without_managed_mode(before), without_managed_mode(after));
        assert!(planned.ends_with("[desktop]\nconversationDetailMode = \"STEPS_COMMANDS\"\n"));
    }

    #[test]
    fn explicit_commands_override_legacy_prose_migration_fixture() {
        // The shipped app's SettingsStore.initialize prefers an explicit
        // config.desktop value over its pending global-state migration.
        let missing = CONFIG_WITH_PROSE.replace(
            "conversationDetailMode = \"STEPS_PROSE\" # preserve this comment\n",
            "",
        );
        let planned = plan_config(&missing, true).unwrap().unwrap();
        let config: Value = planned.parse().unwrap();
        let explicit_config = config
            .get("desktop")
            .and_then(Value::as_table)
            .and_then(|desktop| desktop.get("conversationDetailMode"))
            .and_then(Value::as_str);
        let legacy_global_state = Some(PROSE_MODE);
        assert_eq!(explicit_config, Some(COMMANDS_MODE));
        assert_eq!(explicit_config.or(legacy_global_state), Some(COMMANDS_MODE));
    }

    #[test]
    fn selected_profile_provider_is_respected() {
        let config = r#"profile = "work"

[profiles.work]
model_provider = "custom"

[desktop]
conversationDetailMode = "STEPS_PROSE"

[model_providers.custom]
base_url = "https://404token.xyz/v1"
"#;
        let planned = plan_config(config, true).unwrap().unwrap();
        let after: Value = planned.parse().unwrap();
        assert_eq!(detail_mode(&after), ConfigMode::Commands);
        assert_eq!(
            codex_config_readback::active_model_provider(&after),
            Some("custom")
        );
    }
}
