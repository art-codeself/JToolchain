//! 环境变量持久化。
//!
//! 设计要点：`JAVA_HOME` / `MAVEN_HOME` / `GRADLE_HOME` / `PATH` 里放的都是
//! **固定的** `...\.jtc\current\xxx` 路径，切换版本时只改 current 链接指向，
//! 环境变量本身永不改动。因此无需 shell hook，CMD / PowerShell / GitBash 行为一致。

use std::path::Path;

use anyhow::Result;

/// 需要写入 PATH 的目录。
///
/// 这里刻意写成**展开后的绝对路径**而不是 `%JTC_HOME%\bin`：
/// 并非所有宿主都会展开 PATH 里的变量（部分 IDE 内置终端、老式启动器、
/// WSL 互操作场景就不会），`%JTC_HOME%` 形式在这些环境里会直接找不到命令。
/// 代价是 JTC_HOME 换位置后需重跑一次 `jtc setup`，而它本身就是幂等的。
pub fn path_entries(home: &str) -> Vec<String> {
    let home = home.trim_end_matches(['\\', '/']);
    vec![
        format!("{}\\current\\jdk\\bin", home),
        format!("{}\\current\\maven\\bin", home),
        format!("{}\\current\\gradle\\bin", home),
        format!("{}\\bin", home),
    ]
}

/// 把字符串里的 `%VAR%` 展开成实际值（先查进程环境，再查注册表）。
fn expand_vars(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                let val = std::env::var(name).ok().or_else(|| get(name));
                out.push_str(&val.unwrap_or_default());
                rest = &after[end + 1..];
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// 展开环境变量并转成可直接访问的路径（保留原始大小写），用于判断文件是否存在。
pub fn expand_path(p: &str) -> String {
    let s = p.trim().trim_matches('"');
    msys_to_win(&expand_vars(s))
}

/// 路径归一化：展开环境变量、统一分隔符、去掉尾部斜杠、转小写。
///
/// 同时兼容 MSYS/Git Bash 的 `/c/Users/...` 形式，避免在这类 shell 里
/// 明明配好了却判定为缺失。
pub fn normalize_path(p: &str) -> String {
    let expanded = expand_vars(p.trim());
    let win = msys_to_win(&expanded);
    win.trim_end_matches(['\\', '/']).to_lowercase()
}

/// `/c/Users/x` -> `C:\Users\x`，其余只把正斜杠换成反斜杠。
fn msys_to_win(p: &str) -> String {
    let b = p.as_bytes();
    if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b'/' {
        format!("{}:{}", (b[1] as char).to_ascii_uppercase(), &p[2..].replace('/', "\\"))
    } else {
        p.replace('/', "\\")
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    use anyhow::{Context, Result};
    use winreg::enums::{RegType, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE};
    use winreg::{RegKey, RegValue};

    fn env_key() -> Result<RegKey> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        hkcu.open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)
            .context("打开 HKCU\\Environment 失败")
    }

    /// 系统级环境变量（需要管理员权限才能写）。
    fn machine_env_key(write: bool) -> Result<RegKey> {
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let flags = if write { KEY_READ | KEY_WRITE } else { KEY_READ };
        hklm.open_subkey_with_flags(
            r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
            flags,
        )
        .context("打开系统 Environment 失败（写入系统 PATH 需要管理员权限）")
    }

    fn to_wide(value: &str) -> Vec<u8> {
        let mut v: Vec<u8> = Vec::new();
        for u in value.encode_utf16().chain(std::iter::once(0)) {
            v.extend_from_slice(&u.to_le_bytes());
        }
        v
    }

    fn from_wide(bytes: &[u8]) -> OsString {
        let u16s: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        OsString::from_wide(&u16s)
    }

    pub fn get(name: &str) -> Option<String> {
        let key = env_key().ok()?;
        let raw: RegValue = key.get_raw_value(name).ok()?;
        let s = from_wide(&raw.bytes);
        Some(s.to_string_lossy().trim_end_matches('\0').to_string())
    }

    /// 以 REG_EXPAND_SZ 写入，允许值里保留 `%JTC_HOME%` 这类未展开引用。
    pub fn set_expand(name: &str, value: &str) -> Result<()> {
        let key = env_key()?;
        key.set_raw_value(
            name,
            &RegValue {
                vtype: RegType::REG_EXPAND_SZ,
                bytes: to_wide(value),
            },
        )
        .with_context(|| format!("写入环境变量 {} 失败", name))
    }

    /// 读系统级环境变量。
    pub fn machine_get(name: &str) -> Option<String> {
        let key = machine_env_key(false).ok()?;
        let raw: RegValue = key.get_raw_value(name).ok()?;
        let s = from_wide(&raw.bytes);
        Some(s.to_string_lossy().trim_end_matches('\0').to_string())
    }

    /// 写系统级环境变量（需管理员权限）。
    pub fn machine_set_expand(name: &str, value: &str) -> Result<()> {
        let key = machine_env_key(true)?;
        key.set_raw_value(
            name,
            &RegValue {
                vtype: RegType::REG_EXPAND_SZ,
                bytes: to_wide(value),
            },
        )
        .with_context(|| format!("写入系统环境变量 {} 失败（需要管理员权限）", name))
    }
}

#[cfg(windows)]
pub use imp::*;

#[cfg(windows)]
/// 把 jtc 需要的目录写进用户 PATH，返回是否有改动。
///
/// 做两件事：
/// 1. 先**清理**历史遗留的 jtc 条目（含旧版写入的 `%JTC_HOME%\...` 形式），
///    否则每次 setup 都会重复追加，PATH 越跑越长；
/// 2. 再追加当前 JTC_HOME 对应的绝对路径条目。
pub fn ensure_path_entries(home: &str) -> Result<bool> {
    let entries = path_entries(home);
    let current = imp::get("PATH").unwrap_or_default();
    let home_norm = normalize_path(home);

    let mut kept: Vec<String> = Vec::new();
    for raw in current.split(';') {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let n = normalize_path(raw);
        // 命中 jtc 自己的目录（绝对路径或旧的 %JTC_HOME% 形式）就剔除，稍后统一重写
        if n.contains("%jtc_home%") || n.starts_with(&home_norm) {
            continue;
        }
        kept.push(raw.to_string());
    }

    let mut missing: Vec<String> = Vec::new();
    for e in &entries {
        let n = normalize_path(e);
        if kept.iter().any(|k| normalize_path(k) == n) {
            continue;
        }
        missing.push(e.clone());
    }

    let mut merged = kept.join(";");
    for e in &missing {
        if !merged.is_empty() {
            merged.push(';');
        }
        merged.push_str(e);
    }

    // 用归一化结果比较，避免「内容相同、写法不同」被当成改动而反复写注册表
    if normalize_path(&merged) == normalize_path(&current) {
        return Ok(false);
    }
    imp::set_expand("PATH", &merged)?;
    Ok(true)
}

#[cfg(windows)]
/// 通知系统环境变量已变更，让后续启动的进程能拿到新值。
///
/// 直接链接 user32，避免为此引入庞大的 windows-sys 元数据。
pub fn broadcast() {
    const HWND_BROADCAST: isize = 0xffff;
    const WM_SETTINGCHANGE: u32 = 0x001A;
    const SMTO_ABORTIFHUNG: u32 = 0x0002;

    #[link(name = "user32")]
    extern "system" {
        fn SendMessageTimeoutW(
            hwnd: isize,
            msg: u32,
            wparam: usize,
            lparam: isize,
            flags: u32,
            timeout: u32,
            result: *mut usize,
        ) -> isize;
    }

    unsafe {
        let mut result: usize = 0;
        let env: Vec<u16> = "Environment\0".encode_utf16().collect();
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            env.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            5000,
            &mut result,
        );
    }
}

#[cfg(not(windows))]
mod imp {
    use anyhow::Result;

    pub fn get(name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    pub fn set_expand(_name: &str, _value: &str) -> Result<()> {
        Ok(())
    }

    pub fn machine_get(name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    pub fn machine_set_expand(_name: &str, _value: &str) -> Result<()> {
        Ok(())
    }

    pub fn delete(_name: &str) -> Result<()> {
        Ok(())
    }
}

#[cfg(not(windows))]
pub use imp::*;

/// 把 jtc 的目录写到**系统 PATH 最前面**（需要管理员权限）。
///
/// Windows 组装环境时的顺序是「系统 PATH + 用户 PATH」，而安装 JDK 时安装器会往系统
/// PATH 前面塞 `...\Oracle\Java\javapath`、`...\jdk1.8.0_131\bin` 这类条目。它们会
/// 永久盖住用户 PATH 里的 jtc，于是 `jtc use` 明明切了链接，`java -version` 却不变。
/// 只有提到系统 PATH 最前，切换才真正生效。
#[cfg(windows)]
pub fn promote_to_machine_path(home: &str) -> Result<bool> {
    let entries = path_entries(home);
    let current = imp::machine_get("PATH").unwrap_or_default();
    let home_norm = normalize_path(home);

    let mut kept: Vec<String> = Vec::new();
    for raw in current.split(';') {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let n = normalize_path(raw);
        // 剔除历史遗留的 jtc 条目（含旧 %JTC_HOME% 形式），稍后统一前置重写
        if n.contains("%jtc_home%") || n.starts_with(&home_norm) {
            continue;
        }
        kept.push(raw.to_string());
    }

    let merged: Vec<String> = entries.iter().cloned().chain(kept).collect();
    let new = merged.join(";");
    if normalize_path(&new) == normalize_path(&current) {
        return Ok(false);
    }
    imp::machine_set_expand("PATH", &new)?;
    Ok(true)
}

#[cfg(not(windows))]
pub fn promote_to_machine_path(_home: &str) -> Result<bool> {
    Ok(false)
}

#[cfg(windows)]
/// 组合出新进程实际看到的 PATH：系统 PATH 在前，用户 PATH 在后（Windows 真实顺序）。
pub fn effective_path() -> String {
    let parts = [
        imp::machine_get("PATH").unwrap_or_default(),
        imp::get("PATH").unwrap_or_default(),
    ];
    let mut out = String::new();
    for p in parts {
        for item in p.split(';') {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push(';');
            }
            out.push_str(item);
        }
    }
    out
}

#[cfg(not(windows))]
pub fn effective_path() -> String {
    std::env::var("PATH").unwrap_or_default()
}

#[cfg(not(windows))]
pub fn broadcast() {}

/// 生成供 shell 源码引用的环境变量片段。
pub fn shell_exports(home: &Path, shell: &str) -> String {
    let home_s = home.to_string_lossy().replace('\\', "/");
    let jdk = format!("{}/current/jdk", home_s);
    let maven = format!("{}/current/maven", home_s);
    let gradle = format!("{}/current/gradle", home_s);
    // Windows 的 PATH 只认 `;` 分隔（写 `:` 会让整串变成一个无效条目），
    // bash 才用 `:`；顺带把 Windows 系列的路径统一成反斜杠形式。
    let win = matches!(shell, "cmd" | "powershell");
    let sep = if win { ";" } else { ":" };
    let path_add = format!(
        "{jdk}/bin{sep}{maven}/bin{sep}{gradle}/bin{sep}{home}/bin",
        jdk = jdk,
        maven = maven,
        gradle = gradle,
        home = home_s,
        sep = sep
    );

    let (jdk, maven, gradle, path_add) = if win {
        let w = |p: &str| p.replace('/', "\\");
        (w(&jdk), w(&maven), w(&gradle), w(&path_add))
    } else {
        (jdk, maven, gradle, path_add)
    };

    match shell {
        "cmd" => {
            let w = |p: &str| p.replace('/', "\\");
            format!(
                "set JAVA_HOME={jdk}\r\n\
                 set MAVEN_HOME={maven}\r\n\
                 set M2_HOME={maven}\r\n\
                 set GRADLE_HOME={gradle}\r\n\
                 set PATH={path_add};%PATH%\r\n",
                jdk = w(&jdk),
                maven = w(&maven),
                gradle = w(&gradle),
                path_add = w(&path_add)
            )
        }
        "powershell" => format!(
            "$env:JAVA_HOME = '{jdk}'\n\
             $env:MAVEN_HOME = '{maven}'\n\
             $env:M2_HOME = '{maven}'\n\
             $env:GRADLE_HOME = '{gradle}'\n\
             $env:PATH = '{path_add};' + $env:PATH\n",
            jdk = jdk,
            maven = maven,
            gradle = gradle,
            path_add = path_add
        ),
        _ => format!(
            "export JAVA_HOME=\"{jdk}\"\n\
             export MAVEN_HOME=\"{maven}\"\n\
             export M2_HOME=\"{maven}\"\n\
             export GRADLE_HOME=\"{gradle}\"\n\
             export PATH=\"{path_add}:$PATH\"\n",
            jdk = jdk,
            maven = maven,
            gradle = gradle,
            path_add = path_add
        ),
    }
}
