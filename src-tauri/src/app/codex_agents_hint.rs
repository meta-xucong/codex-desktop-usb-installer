use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const MARKER_BEGIN: &str = "# BEGIN CODEX-USB-I18N-MOJIBAKE-HINT v1";
const MARKER_END: &str = "# END CODEX-USB-I18N-MOJIBAKE-HINT v1";
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
- 对大文件只输出相关片段，避免反复将整份乱码内容送入上下文。"#;

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
    let body = HINT_BODY.replace('\n', newline);
    format!("{MARKER_BEGIN}{newline}{newline}{body}{newline}{newline}{MARKER_END}")
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
