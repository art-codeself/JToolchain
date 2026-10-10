//! 工具抽象：JDK / Maven / Gradle 三类共用的本地管理逻辑 + Provider 接口。

pub mod gradle;
pub mod jdk;
pub mod maven;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::config::{Config, Paths};
use crate::fsutil;

/// 被管理的工具类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Jdk,
    Maven,
    Gradle,
}

impl Kind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Jdk => "jdk",
            Kind::Maven => "maven",
            Kind::Gradle => "gradle",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Kind::Jdk => "JDK",
            Kind::Maven => "Maven",
            Kind::Gradle => "Gradle",
        }
    }

    /// current 链接对应的环境变量名。
    pub fn home_var(&self) -> &'static str {
        match self {
            Kind::Jdk => "JAVA_HOME",
            Kind::Maven => "MAVEN_HOME",
            Kind::Gradle => "GRADLE_HOME",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        match s.to_ascii_lowercase().as_str() {
            "jdk" | "java" => Some(Kind::Jdk),
            "maven" | "mvn" => Some(Kind::Maven),
            "gradle" => Some(Kind::Gradle),
            _ => None,
        }
    }

    pub fn all() -> [Kind; 3] {
        [Kind::Jdk, Kind::Maven, Kind::Gradle]
    }
}

/// 远端可安装的一个版本。
#[derive(Debug, Clone)]
pub struct Remote {
    /// 安装后的目录名，如 `21.0.5+11`
    pub version: String,
    /// 下载链接（已按镜像改写）
    pub url: String,
    /// 官方源地址，镜像失效时自动回退到这里
    pub official: String,
    /// 附加说明（LTS、归档等）
    pub note: String,
}

pub trait Provider {
    #[allow(dead_code)]
    fn kind(&self) -> Kind;
    /// 远端可用版本列表（用于 ls-remote）。
    fn list_remote(&self, cfg: &Config) -> Result<Vec<Remote>>;
    /// 把用户输入的版本请求解析成确定的远端版本。
    fn resolve(&self, req: &str, cfg: &Config) -> Result<Remote>;
}

pub fn provider(kind: Kind) -> Box<dyn Provider> {
    match kind {
        Kind::Jdk => Box::new(jdk::Jdk),
        Kind::Maven => Box::new(maven::Maven),
        Kind::Gradle => Box::new(gradle::Gradle),
    }
}

// ---------------- 本地已安装版本管理 ----------------

/// 已安装版本目录名列表，按版本号升序。
pub fn installed(paths: &Paths, kind: Kind) -> Vec<String> {
    let dir = paths.tool_dir(kind.as_str());
    let mut vers: Vec<String> = match fs::read_dir(&dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| !n.starts_with('.'))
            .collect(),
        Err(_) => Vec::new(),
    };
    vers.sort_by(|a, b| cmp_version(a, b));
    vers
}

/// 宽松的版本解析：把 `8.5` 这类补全成 `8.5.0` 以便比较。
fn parse_version(v: &str) -> (u64, u64, u64, String) {
    let core = v.split('+').next().unwrap_or(v);
    let build = v.split_once('+').map(|(_, b)| b.to_string()).unwrap_or_default();
    let parts: Vec<&str> = core.split(['.', '-', '_']).collect();
    let num = |i: usize| {
        parts
            .get(i)
            .and_then(|s| s.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok())
            .unwrap_or(0)
    };
    (num(0), num(1), num(2), build)
}

pub fn cmp_version(a: &str, b: &str) -> std::cmp::Ordering {
    parse_version(a).cmp(&parse_version(b))
}

/// 模糊匹配已安装版本：
/// - 完全相等优先
/// - 否则按「前缀 + 边界符」匹配，取版本最高者（如 `21` -> `21.0.5+11`）
pub fn match_installed(paths: &Paths, kind: Kind, req: &str) -> Result<String> {
    let vers = installed(paths, kind);
    if vers.is_empty() {
        bail!(
            "尚未安装任何 {}，先执行 `jtc {} install <version>`",
            kind.label(),
            kind.as_str()
        );
    }
    if let Some(exact) = vers.iter().find(|v| v.as_str() == req) {
        return Ok(exact.clone());
    }
    let mut hits: Vec<&String> = vers
        .iter()
        .filter(|v| {
            v.starts_with(req)
                && (v.len() == req.len()
                    || matches!(
                        v.as_bytes().get(req.len()).copied(),
                        Some(b'.') | Some(b'+') | Some(b'-') | Some(b'_')
                    ))
        })
        .collect();
    if hits.is_empty() {
        bail!(
            "没有匹配的已安装 {} 版本: {}\n已安装: {}",
            kind.label(),
            req,
            vers.join(", ")
        );
    }
    hits.sort_by(|a, b| cmp_version(b, a));
    Ok(hits[0].clone())
}

/// 当前生效版本（读 current 链接）。
pub fn current(paths: &Paths, kind: Kind) -> Option<String> {
    fsutil::linked_version(&paths.current_link(kind.as_str()))
}

/// 切换版本：只重建 current 链接，环境变量完全不动。
pub fn use_version(paths: &Paths, kind: Kind, version: &str) -> Result<()> {
    let target = paths.version_dir(kind.as_str(), version);
    if !target.is_dir() {
        bail!(
            "{} {} 未安装（目录不存在: {}）",
            kind.label(),
            version,
            target.display()
        );
    }
    fsutil::link_dir(&target, &paths.current_link(kind.as_str()))
        .with_context(|| format!("切换 {} 到 {} 失败", kind.label(), version))?;
    Ok(())
}

/// 卸载：若正被 current 指向，需先处理链接。
///
/// 通过 `jtc link` 登记进来的版本本身是个链接，这里只删链接，
/// 绝不触碰用户原有的安装目录。
pub fn uninstall(paths: &Paths, kind: Kind, version: &str) -> Result<()> {
    let dir = paths.version_dir(kind.as_str(), version);
    if !dir.is_dir() {
        bail!("{} {} 未安装", kind.label(), version);
    }
    let was_current = current(paths, kind).as_deref() == Some(version);

    if fsutil::is_link(&dir) {
        fsutil::remove_link(&dir)?;
    } else {
        fsutil::remove_dir_all_safe(&dir)?;
    }

    if was_current {
        let link = paths.current_link(kind.as_str());
        fsutil::remove_link(&link)?;
        // 自动回退到剩余的最高版本，避免 JAVA_HOME 指向空目录
        let rest = installed(paths, kind);
        if let Some(fallback) = rest.last() {
            use_version(paths, kind, fallback)?;
            return Ok(());
        }
    }
    Ok(())
}

/// 当前生效的可执行文件绝对路径。
pub fn current_exe(paths: &Paths, kind: Kind, exe: &str) -> Option<std::path::PathBuf> {
    let base = paths.current_link(kind.as_str());
    if !base.exists() {
        return None;
    }
    let bin = base.join("bin");
    for name in exe_candidates(exe) {
        let p = bin.join(name);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

pub fn exe_candidates(exe: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        let lower = exe.to_ascii_lowercase();
        let base = lower.trim_end_matches(".exe");
        let mut v = vec![
            format!("{}.exe", base),
            format!("{}.cmd", base),
            format!("{}.bat", base),
        ];
        v.push(exe.to_string());
        v
    }
    #[cfg(not(windows))]
    {
        vec![exe.to_string()]
    }
}

/// 校验某个目录是否真的是该类工具的安装根目录。
pub fn validate_dir(kind: Kind, dir: &Path) -> Result<()> {
    if !dir.is_dir() {
        bail!("目录不存在: {}", dir.display());
    }
    let bin = dir.join("bin");
    // JDK 要求 java + javac 都有，这样纯 JRE 不会被误当成 JDK 登记进来
    let need: &[&str] = match kind {
        Kind::Jdk => &["java", "javac"],
        Kind::Maven => &["mvn"],
        Kind::Gradle => &["gradle"],
    };
    for n in need {
        let found = exe_candidates(n).iter().any(|c| bin.join(c).exists())
            || bin.join(n).exists();
        if !found {
            bail!(
                "不像 {} 的安装目录（缺少 bin/{}）: {}",
                kind.label(),
                n,
                dir.display()
            );
        }
    }
    Ok(())
}

/// 从目录名推断版本号：`jdk1.8.0_131` -> `1.8.0_131`，`apache-maven-3.9.6` -> `3.9.6`
pub fn version_from_name(name: &str) -> Option<String> {
    let start = name.find(|c: char| c.is_ascii_digit())?;
    let rest = &name[start..];
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '_' | '+' | '-')))
        .unwrap_or(rest.len());
    let v = rest[..end].trim_end_matches(['.', '-', '+']);
    if v.is_empty() || !v.chars().any(|c| c.is_ascii_digit()) {
        None
    } else {
        Some(v.to_string())
    }
}

/// 本机常见安装位置（用于 `jtc <tool> scan`）。
pub fn scan_roots(kind: Kind) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut add = |p: PathBuf| {
        if p.is_dir() {
            roots.push(p);
        }
    };

    #[cfg(windows)]
    {
        let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| "C:\\Program Files".into());
        let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
        let home = dirs::home_dir().unwrap_or_default();

        match kind {
            Kind::Jdk => {
                for d in [
                    "Java",
                    "Eclipse Adoptium",
                    "Amazon Corretto",
                    "Azul Systems",
                    "Zulu",
                    "Microsoft",
                    "BellSoft",
                    "GraalVM",
                    "RedHat",
                    "Semeru",
                ] {
                    add(Path::new(&pf).join(d));
                }
                add(Path::new(&local).join("Programs").join("Java"));
                add(home.join(".sdkman").join("candidates").join("java"));
                add(home.join("scoop").join("apps"));
            }
            Kind::Maven => {
                add(PathBuf::from(&pf));
                add(home.join("scoop").join("apps").join("maven"));
                add(PathBuf::from(r"C:\ProgramData\chocolatey\lib\maven\tools"));
                add(home.join(".sdkman").join("candidates").join("maven"));
            }
            Kind::Gradle => {
                add(PathBuf::from(&pf));
                add(home.join("scoop").join("apps").join("gradle"));
                add(PathBuf::from(r"C:\ProgramData\chocolatey\lib\gradle\tools"));
                add(home.join(".sdkman").join("candidates").join("gradle"));
                add(PathBuf::from(r"C:\Gradle"));
            }
        }
    }

    #[cfg(not(windows))]
    {
        let home = dirs::home_dir().unwrap_or_default();
        match kind {
            Kind::Jdk => {
                for d in [
                    "/usr/lib/jvm",
                    "/Library/Java/JavaVirtualMachines",
                    "/opt",
                    "/usr/local/opt",
                ] {
                    add(PathBuf::from(d));
                }
                add(home.join(".sdkman").join("candidates").join("java"));
            }
            Kind::Maven => {
                for d in ["/opt", "/usr/local", "/usr/share"] {
                    add(PathBuf::from(d));
                }
                add(home.join(".sdkman").join("candidates").join("maven"));
            }
            Kind::Gradle => {
                for d in ["/opt", "/usr/local"] {
                    add(PathBuf::from(d));
                }
                add(home.join(".sdkman").join("candidates").join("gradle"));
            }
        }
    }

    roots
}

/// 找到任意 Kind（用于 `jtc which`）。
pub fn find_any(paths: &Paths, exe: &str) -> Option<(Kind, std::path::PathBuf)> {
    Kind::all()
        .iter()
        .find_map(|k| current_exe(paths, *k, exe).map(|p| (*k, p)))
        .or_else(|| {
            let map: Vec<(Kind, &str)> = vec![
                (Kind::Jdk, "java"),
                (Kind::Jdk, "javac"),
                (Kind::Maven, "mvn"),
                (Kind::Gradle, "gradle"),
            ];
            let lower = exe.to_ascii_lowercase();
            map.iter()
                .find(|(_, n)| *n == lower.trim_end_matches(".exe"))
                .and_then(|(k, n)| current_exe(paths, *k, n).map(|p| (*k, p)))
        })
}

/// HEAD 请求，判断 URL 是否可下载。
pub fn url_exists(url: &str, cfg: &Config) -> bool {
    let agent = ureq::AgentBuilder::new()
        .try_proxy_from_env(true)
        .timeout_connect(std::time::Duration::from_secs(cfg.timeout_secs))
        .build();
    agent.head(url).call().map(|r| r.status() == 200).unwrap_or(false)
}

