//! Restore only the capability boundary lost by the former Node-table updater.
//! The existing capability renderer/merger consumes this marker before installation.
use std::{fs, io, path::Path};

const BEGIN: &str = "# BEGIN CODEX-STARTER-KIT MANAGED CONFIG v1";
const END: &str = "# END CODEX-STARTER-KIT MANAGED CONFIG v1";

fn marker_offsets(text: &str, marker: &str) -> Vec<usize> {
    let mut offset = 0;
    let mut result = Vec::new();
    for line in text.split_inclusive('\n') {
        if line.trim_end_matches('\n') == marker {
            result.push(offset);
        }
        offset += line.len();
    }
    result
}

fn restore_from_backups(current: &str, backups: &[String]) -> Result<Option<String>, &'static str> {
    let normalized = current.replace("\r\n", "\n");
    let begins = marker_offsets(&normalized, BEGIN);
    let ends = marker_offsets(&normalized, END);
    if !begins.is_empty() || ends.len() != 1 {
        return Ok(None);
    }
    let end = ends[0];
    let mut matched_start = None;
    for backup in backups {
        let backup = backup.replace("\r\n", "\n");
        let begin = marker_offsets(&backup, BEGIN);
        let finish = marker_offsets(&backup, END);
        if begin.len() != 1 || finish.len() != 1 || begin[0] >= finish[0] {
            continue;
        }
        let mut body = &backup[begin[0] + BEGIN.len() + 1..finish[0]];
        while let Some(line) = body.split_inclusive('\n').next() {
            if !line.trim().is_empty() && !line.trim_start().starts_with('#') {
                break;
            }
            body = &body[line.len()..];
        }
        if body.lines().next() != Some("[[skills.config]]") || !normalized[..end].ends_with(body) {
            continue;
        }
        let start = end - body.len();
        if start > 0 && normalized.as_bytes()[start - 1] != b'\n' {
            continue;
        }
        if matched_start.is_some_and(|previous| previous != start) {
            return Err("能力包边界缺失，但历史备份无法确定唯一恢复位置；现有配置未覆盖。");
        }
        matched_start = Some(start);
    }
    let start =
        matched_start.ok_or("能力包边界缺失，且未找到内容完全匹配的完整备份；现有配置未覆盖。")?;
    let line_number = normalized[..start]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count();
    let raw_offset: usize = current
        .split_inclusive('\n')
        .take(line_number)
        .map(str::len)
        .sum();
    let newline = if current[raw_offset..]
        .split_inclusive('\n')
        .next()
        .is_some_and(|line| line.ends_with("\r\n"))
    {
        "\r\n"
    } else {
        "\n"
    };
    let restored = format!(
        "{}{BEGIN}{newline}{}",
        &current[..raw_offset],
        &current[raw_offset..]
    );
    let parse = |text: &str| text.trim_start_matches('\u{feff}').parse::<toml::Value>();
    let before = parse(current).map_err(|_| "能力包边界恢复前的配置无法解析；现有配置未覆盖。")?;
    if parse(&restored).ok().as_ref() != Some(&before) {
        return Err("能力包边界恢复未通过配置值保留检查；现有配置未覆盖。");
    }
    Ok(Some(restored))
}

pub(crate) fn restore_for_install(home: &Path) -> Result<bool, String> {
    let path = home.join("config.toml");
    let current = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("无法读取能力包配置：{error}")),
    };
    let normalized = current.replace("\r\n", "\n");
    if !marker_offsets(&normalized, BEGIN).is_empty() || marker_offsets(&normalized, END).len() != 1
    {
        return Ok(false);
    }
    let backup_root = home.join("codex-agent-kit/backups");
    let mut backups = Vec::new();
    for entry in
        fs::read_dir(backup_root).map_err(|error| format!("无法读取能力包配置备份目录：{error}"))?
    {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("config.toml.")
            && name.ends_with(".bak")
            && entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_file()
        {
            backups.push(fs::read_to_string(entry.path()).map_err(|error| error.to_string())?);
        }
    }
    let Some(restored) = restore_from_backups(&current, &backups).map_err(str::to_owned)? else {
        return Ok(false);
    };
    // Preserve the original configuration using the installer's existing backup directory.
    let backup = home.join("codex-agent-kit/backups/config.toml.before-managed-begin-recovery.bak");
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&backup)
        .and_then(|mut file| {
            use std::io::Write;
            file.write_all(current.as_bytes())
        })
        .map_err(|error| format!("无法保存边界恢复前的配置备份：{error}"))?;
    if fs::read_to_string(&path).map_err(|error| error.to_string())? != current {
        return Err("恢复期间配置被其他进程更新；现有配置未覆盖。".into());
    }
    crate::app::atomic_file::write_atomic_preserving_backup(&path, restored.as_bytes())
        .map_err(|error| format!("无法恢复能力包配置边界：{error}"))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    const BODY: &str = "[[skills.config]]\npath = 'fixture/SKILL.md'\nenabled = true\n[mcp_servers.cloudflare]\nenabled = false\n";
    fn backup() -> String {
        format!("model = 'old'\n{BEGIN}\n# generated\n{BODY}{END}\n")
    }
    fn damaged() -> String {
        format!("model = 'current'\n{BODY}{END}\n[mcp_servers.node_repl_local]\nenabled = true\n")
    }
    #[test]
    fn restores_only_comment_and_keeps_current_values() {
        let current = damaged();
        let restored = restore_from_backups(&current, &[backup(), backup()])
            .unwrap()
            .unwrap();
        assert_eq!(restored.replace(&format!("{BEGIN}\n"), ""), current);
        assert!(restored.starts_with("model = 'current'"));
        assert!(restore_from_backups(&restored, &[backup()])
            .unwrap()
            .is_none());
    }
    #[test]
    fn preserves_crlf_and_rejects_nonmatching_content() {
        let current = damaged().replace('\n', "\r\n");
        let restored = restore_from_backups(&current, &[backup()])
            .unwrap()
            .unwrap();
        assert_eq!(restored.replace(&format!("{BEGIN}\r\n"), ""), current);
        assert!(restore_from_backups(
            &damaged().replace("enabled = false", "enabled = true"),
            &[backup()]
        )
        .is_err());
        assert!(restore_from_backups(&damaged(), &[]).is_err());
    }
    #[test]
    fn complete_and_unmanaged_configs_are_unchanged() {
        assert!(restore_from_backups(&backup(), &[]).unwrap().is_none());
        assert!(restore_from_backups("model = 'custom'\n", &[])
            .unwrap()
            .is_none());
    }
}
