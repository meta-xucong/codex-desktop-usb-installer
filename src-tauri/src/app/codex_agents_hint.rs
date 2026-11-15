use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const MARKER_BEGIN: &str = "# BEGIN CODEX-USB-I18N-MOJIBAKE-HINT v1";
const MARKER_END: &str = "# END CODEX-USB-I18N-MOJIBAKE-HINT v1";
const RUNTIME_GUIDANCE: &str = include_str!("usb_runtime_guidance.md");
const HINT_BODY: &str = r#"## 中文输出乱码处理（仅在出现时使用）

若 exec 读取 UTF-8 中文文件时出现「閫氳繃」「鏄惧紡」一类明显乱码，不要反复重试或把整份大文件重复输出。先确认源文件编码和当前 exec 输出解码方式；只在已确认 Windows 输出按系统代码页 936 解码时，使用下面的 PowerShell 方法：

[Console]::OutputEncoding=[Text.Encoding]::GetEncoding(936); $p='文件路径'; $t=[Text.Encoding]::UTF8.GetString([IO.File]::ReadAllBytes($p)); $t=[regex]::Replace($t,'[\uD800-\uDBFF][\uDC00-\uDFFF]','?'); $t

短文本可尝试还原：
$m='乱码文本'; [Text.Encoding]::UTF8.GetString([Text.Encoding]::GetEncoding(936).GetBytes($m))

说明：
- 该方法针对已确认的「UTF-8 内容被按代码页 936 解码」场景，不适用于 UTF-8 控制台或其他代码页。
- 读取文件字节后显式按 UTF-8 解码，并让 PowerShell 按 936 输出；仅指定 `-Encoding UTF8` 不能修复后续输出解码不匹配。
- 不要在此场景下把 `[Console]::OutputEncoding` 改成 UTF-8，否则接收端若仍按 936 解码会继续乱码。
- 为避免非基本多文种字符在 936 下错位，命令会将 UTF-16 代理项对替换为 `?`；正文可读，emoji 等补充字符会显示为问号。
- 如果环境并非代码页 936，先使用工具原生文件读取能力或识别出的实际编码，不要套用上述转换。
- 对大文件只输出相关片段，避免反复将整份乱码内容送入上下文。

## 图片与视频生成的默认方式

首次使用 U 盘默认媒体入口生成/编辑图片、生成视频或检查该入口是否可用时，自行完成下面的发现和只读能力查询，再继续用户任务。不要先查询 CUA 应用/浏览器清单，也不要让用户替你查工具、复制命令或重新安装。本会话已有成功的媒体工具查询时可沿用；用户只要提示词/方案或明确指定其他生成方式时不做此探测。

若当轮 `functions.exec` 声明提供 `ALL_TOOLS`，先在其中执行：

<!-- snippet: media-entry-discovery -->
```js
text(ALL_TOOLS.filter(tool =>
  /media_generation|media_capabilities/i.test(tool.name + "\n" + tool.description)
));
```

读取匹配工具的真实声明，然后用返回的注册名调用只读 `media_capabilities({})`；当前包常见注册名是 `mcp__media_generation__media_capabilities`，通过 `functions.exec` 的 `tools` 调用，不能因顶层没有 `media` 就跳过。实际调用及安全摘要示例在本轮技能目录的 `image-2/SKILL.md`。不要用脚本或 HTTP 代替会话 MCP 验证。

只有本轮完整发现确实无匹配，才能报告“当前会话未发现媒体工具”；不能据此说文件未安装。发现后调用报错，要报告调用错误；查询成功但目录为 `not_checked`/`stale`，表示仍需目录检查，不表示 MCP 缺失。查询成功就继续原任务，不再要求用户确认工具是否存在。

当用户要求实际生成或编辑图片、生成视频时，默认使用本机已安装的统一 `media_generation` MCP：先检查 `media_capabilities`，再按 MCP 工作流准备、展示确认信息，并在用户确认后提交。不要先尝试 Codex 原生 ImageGen、其他图片/视频工具、外部脚本或直连 API，也不要只输出提示词来代替实际生成。

只有用户在当前对话中明确指定其他生成方式时，才按其指定方式执行。若统一媒体 MCP 或所需模型未就绪，先说明当前不可用；不要悄悄切换到其他生成方式。若用户只要求提示词、分镜或方案，则只交付文字，不提交生成任务。

MCP 的 `media_prepare`、`media_submit`、`media_check`、`media_save` 负责费用确认、单次提交、异步查询和结果保存。提交前遵守 MCP 的确认步骤；未知提交不得重试。"#;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Install the shared hint into the Codex home used by the portable USB build.
/// Returns `Ok(false)` outside USB mode or when the file already has this block.
pub fn ensure_portable_usb_hint() -> io::Result<bool> {
    if !crate::app::usb_guard::guard_required() {
        return Ok(false);
    }
    let paths = crate::app::usb_guard::local_data_paths()
        .map_err(|error| io::Error::other(error.to_string()))?;
    ensure_hint_at(&paths.codex_home)
}

fn ensure_hint_at(codex_home: &Path) -> io::Result<bool> {
    fs::create_dir_all(codex_home)?;
    let path = codex_home.join("AGENTS.md");
    let original = match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Codex AGENTS.md is a symbolic link; leaving it unchanged",
            ));
        }
        Ok(metadata) if !metadata.is_file() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Codex AGENTS.md is not a regular file; leaving it unchanged",
            ));
        }
        Ok(_) => Some(fs::read(&path)?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };

    let existing = match original.as_deref() {
        Some(bytes) => std::str::from_utf8(bytes).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Codex AGENTS.md is not valid UTF-8; leaving it unchanged",
            )
        })?,
        None => "",
    };
    let newline = if existing.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let block = render_block(newline);
    let Some(next) = merge_owned_block(existing, &block, newline)? else {
        return Ok(false);
    };

    write_replace_if_unchanged(&path, original.as_deref(), next.as_bytes())?;
    Ok(true)
}

fn render_block(newline: &str) -> String {
    let body = format!("{}\n\n{HINT_BODY}", RUNTIME_GUIDANCE.trim_end()).replace('\n', newline);
    format!("{MARKER_BEGIN}{newline}{newline}{body}{newline}{newline}{MARKER_END}")
}

#[cfg(test)]
mod runtime_guidance_tests {
    use super::*;

    #[test]
    fn upgrade_preserves_user_text_and_replaces_only_owned_block() {
        let old =
            format!("user before\r\n{MARKER_BEGIN}\r\nold hint\r\n{MARKER_END}\r\nuser after\r\n");
        let block = render_block("\r\n");
        let next = merge_owned_block(&old, &block, "\r\n").unwrap().unwrap();
        assert_eq!(next, format!("user before\r\n{block}\r\nuser after\r\n"));
        assert!(next.contains(&HINT_BODY.replace('\n', "\r\n")));
        assert!(merge_owned_block(&next, &block, "\r\n").unwrap().is_none());
    }

    #[test]
    fn append_preserves_bom_and_unowned_instructions() {
        let original = "\u{feff}# My instructions\nKeep my workflow.";
        let block = render_block("\n");
        let next = merge_owned_block(original, &block, "\n").unwrap().unwrap();
        assert_eq!(next, format!("{original}\n\n{block}\n"));
    }

    #[test]
    fn incomplete_or_duplicate_markers_do_not_produce_a_replacement() {
        let block = render_block("\n");
        for original in [MARKER_BEGIN.to_string(), format!("{block}\n{block}")] {
            assert!(merge_owned_block(&original, &block, "\n").is_err());
        }
    }

    #[test]
    fn installation_is_idempotent_and_leaves_override_and_config_untouched() {
        let root = std::env::temp_dir().join(format!(
            "usb-guidance-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let user_prefix = "# User instructions\r\nKeep my workflow.\r\n";
        fs::write(
            root.join("AGENTS.md"),
            format!("{user_prefix}{MARKER_BEGIN}\r\nold owned hint\r\n{MARKER_END}\r\n"),
        )
        .unwrap();
        fs::write(root.join("AGENTS.override.md"), b"User override wins.\r\n").unwrap();
        fs::write(root.join("config.toml"), b"model = \"fixture\"\n").unwrap();
        assert!(ensure_hint_at(&root).unwrap());
        assert_eq!(
            fs::read_to_string(root.join("AGENTS.md")).unwrap(),
            format!("{user_prefix}{}\r\n", render_block("\r\n"))
        );
        assert!(!ensure_hint_at(&root).unwrap());
        assert_eq!(
            fs::read(root.join("AGENTS.override.md")).unwrap(),
            b"User override wins.\r\n"
        );
        assert_eq!(
            fs::read(root.join("config.toml")).unwrap(),
            b"model = \"fixture\"\n"
        );
    }

    #[test]
    fn concurrent_user_edit_is_preserved() {
        let root = std::env::temp_dir().join(format!(
            "usb-guidance-race-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("AGENTS.md");
        fs::write(&path, b"new user edit").unwrap();
        let error =
            write_replace_if_unchanged(&path, Some(b"old user edit"), b"replacement").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(fs::read(path).unwrap(), b"new user edit");
    }
}

fn merge_owned_block(existing: &str, block: &str, newline: &str) -> io::Result<Option<String>> {
    let begins = existing
        .match_indices(MARKER_BEGIN)
        .map(|(at, _)| at)
        .collect::<Vec<_>>();
    let ends = existing
        .match_indices(MARKER_END)
        .map(|(at, _)| at)
        .collect::<Vec<_>>();

    match (begins.as_slice(), ends.as_slice()) {
        ([], []) => {
            let mut next = existing.to_string();
            if !next.is_empty() {
                if !next.ends_with('\n') {
                    next.push_str(newline);
                }
                next.push_str(newline);
            }
            next.push_str(block);
            next.push_str(newline);
            Ok((next != existing).then_some(next))
        }
        ([begin], [end]) if begin < end => {
            let after_end = *end + MARKER_END.len();
            let mut next = String::with_capacity(existing.len() + block.len());
            next.push_str(&existing[..*begin]);
            next.push_str(block);
            next.push_str(&existing[after_end..]);
            Ok((next != existing).then_some(next))
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Codex AGENTS.md has an incomplete or duplicate USB hint block; leaving it unchanged",
        )),
    }
}

fn write_replace_if_unchanged(
    path: &Path,
    original: Option<&[u8]>,
    bytes: &[u8],
) -> io::Result<()> {
    let (temp, mut file) = create_temp_file(path)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);

        let current = match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Codex AGENTS.md became a symbolic link; leaving it unchanged",
                ));
            }
            Ok(metadata) if !metadata.is_file() => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Codex AGENTS.md is no longer a regular file; leaving it unchanged",
                ));
            }
            Ok(_) => Some(fs::read(path)?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        if current.as_deref() != original {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "Codex AGENTS.md changed while the USB hint was being prepared; leaving it unchanged",
            ));
        }
        fs::rename(&temp, path)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn create_temp_file(path: &Path) -> io::Result<(PathBuf, File)> {
    for _ in 0..32 {
        let temp = temp_path(path)?;
        match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => return Ok((temp, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique AGENTS.md temporary file",
    ))
}

fn temp_path(path: &Path) -> io::Result<PathBuf> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Codex AGENTS.md has no parent directory",
        )
    })?;
    let filename = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Codex AGENTS.md has no filename",
        )
    })?;
    let mut name = filename.to_os_string();
    name.push(format!(
        ".codex-usb-hint-{}-{}.tmp",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    Ok(parent.join(name))
}
