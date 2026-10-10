//! 文件系统工具：目录链接（Windows junction / Unix symlink）与归档解压。

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

/// 创建指向目录的链接。
///
/// - Windows 使用 `mklink /J`（junction）：不需要管理员权限，且被 `java.exe`、
///   `mvn.cmd` 等一切程序视为普通目录，兼容性远优于符号链接。
/// - Unix 使用标准 symlink。
pub fn link_dir(target: &Path, link: &Path) -> Result<()> {
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent)?;
    }
    let _ = remove_link(link);

    #[cfg(windows)]
    {
        // 关键点：
        // 1. mklink 会把路径里的 `/` 当成命令行开关（曾因此报「无效参数 - Users」），
        //    所以必须先统一成反斜杠；
        // 2. 用 raw_arg 自己拼引号，避免 Rust 默认的转义把引号破坏，
        //    这样用户名带空格也能正常工作。
        use std::os::windows::process::CommandExt;

        let cmdline = format!(
            "mklink /J \"{}\" \"{}\"",
            win_path(link),
            win_path(target)
        );
        let out = Command::new("cmd")
            .arg("/c")
            .raw_arg(&cmdline)
            .output()
            .context("调用 mklink 失败")?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let stdout = String::from_utf8_lossy(&out.stdout);
            bail!(
                "创建目录链接失败: {}\n{}\n提示：若系统禁用了 junction，请以管理员身份运行一次 `jtc setup --force`",
                stdout.trim(),
                stderr.trim()
            );
        }
    }

    #[cfg(not(windows))]
    {
        std::os::unix::fs::symlink(target, link)
            .with_context(|| format!("创建符号链接失败: {} -> {}", link.display(), target.display()))?;
    }

    Ok(())
}

/// 把路径统一成 Windows 反斜杠形式（mklink 要求）。
#[cfg(windows)]
fn win_path(p: &Path) -> String {
    p.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_string()
}

/// 删除链接（或空目录），不影响目标目录内容。
pub fn remove_link(link: &Path) -> Result<()> {
    if !link.exists() && !is_link(link) {
        return Ok(());
    }
    // Windows junction 表现为目录；Unix symlink 表现为文件。
    match fs::remove_dir(link) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => match fs::remove_file(link) {
            Ok(()) => Ok(()),
            Err(e2) if e2.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(e).with_context(|| format!("删除链接失败: {}", link.display())),
        },
    }
}

/// 是否是链接（junction / symlink）。
pub fn is_link(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(md) => md.file_type().is_symlink() || (!path.exists() && md.is_dir()),
        Err(_) => false,
    }
}

/// 读取链接指向的目标；非链接返回 None。
pub fn link_target(link: &Path) -> Option<PathBuf> {
    fs::read_link(link).ok().or_else(|| {
        // Windows junction 可能读不到，退回到规范化路径判断
        fs::canonicalize(link).ok()
    })
}

/// 链接当前的版本名（如 `21.0.5+11`），未设置返回 None。
pub fn linked_version(link: &Path) -> Option<String> {
    let target = link_target(link)?;
    let name = target.file_name()?.to_string_lossy().to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// 解压 zip / tar.gz / tgz 到 `dest`，自动剥离归档中唯一的顶层目录。
///
/// 采用「先解到临时目录，再整体改名」的方式，避免中途失败留下半个安装目录。
pub fn extract_archive(archive: &Path, dest: &Path) -> Result<()> {
    let parent = dest
        .parent()
        .context("安装目标路径非法")?
        .to_path_buf();
    let tmp = parent.join(format!(".jtc-extract-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp)?;

    let result = (|| -> Result<()> {
        let lower = archive.to_string_lossy().to_lowercase();
        if lower.ends_with(".zip") {
            unzip(archive, &tmp)?;
        } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
            untar_gz(archive, &tmp)?;
        } else {
            bail!("暂不支持的归档格式: {}", archive.display());
        }

        // 剥离唯一的顶层目录，使 dest 直接就是工具根目录
        let entries: Vec<_> = fs::read_dir(&tmp)?.filter_map(|e| e.ok()).collect();
        let src = if entries.len() == 1 {
            let only = entries[0].path();
            if only.is_dir() {
                only
            } else {
                tmp.clone()
            }
        } else {
            tmp.clone()
        };

        if src == tmp {
            fs::rename(&tmp, dest)?;
        } else {
            fs::rename(&src, dest)?;
            let _ = fs::remove_dir_all(&tmp);
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_dir_all(&tmp);
        let _ = fs::remove_dir_all(dest);
    }
    result
}

fn unzip(archive: &Path, dest: &Path) -> Result<()> {
    let file = fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file).context("打开 zip 归档失败")?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let out_path = match entry.enclosed_name() {
            Some(p) => dest.join(p),
            None => continue, // 路径逃逸项，跳过
        };

        if entry.is_dir() {
            fs::create_dir_all(&out_path)?;
            continue;
        }

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out = fs::File::create(&out_path)
            .with_context(|| format!("创建文件失败: {}", out_path.display()))?;
        io::copy(&mut entry, &mut out)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = entry.unix_mode() {
                fs::set_permissions(&out_path, fs::Permissions::from_mode(mode))?;
            }
        }
        #[cfg(windows)]
        {
            let _ = &mut out;
        }
    }
    Ok(())
}

fn untar_gz(archive: &Path, dest: &Path) -> Result<()> {
    let file = fs::File::open(archive)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(gz);
    tar.unpack(dest).context("解压 tar.gz 失败")?;
    Ok(())
}

/// 递归删除目录；Windows 上 junction 不会被递归到目标内部。
pub fn remove_dir_all_safe(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    fs::remove_dir_all(path).with_context(|| format!("删除目录失败: {}", path.display()))
}

/// 把 `from` 里的全部内容移动到 `to`（同分区内改名，速度快）。
pub fn write_string(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut f = fs::File::create(path)?;
    f.write_all(content.as_bytes())?;
    Ok(())
}

/// 判断目录是否是有效的 JDK 根目录（含 bin/java）。
pub fn looks_like_jdk(dir: &Path) -> bool {
    dir.join("bin").join("java").exists()
        || dir.join("bin").join("java.exe").exists()
}
