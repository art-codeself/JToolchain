//! 项目级配置 `.jtoolchain`：把版本锁定到代码仓库里，团队共用同一套工具链。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::tool::Kind;

pub const FILE_NAME: &str = ".jtoolchain";

/// 从 `from` 开始向上查找 `.jtoolchain`，遇到 `.git` 或磁盘根目录停止。
pub fn find(from: &Path) -> Option<PathBuf> {
    let mut dir = Some(from);
    while let Some(d) = dir {
        let candidate = d.join(FILE_NAME);
        if candidate.is_file() {
            return Some(candidate);
        }
        if d.join(".git").exists() {
            // 到达仓库根仍未找到就停止，避免跨出项目边界
            return None;
        }
        dir = d.parent();
    }
    None
}

/// 解析配置文件，返回 kind -> 版本。
pub fn read(path: &Path) -> Result<HashMap<Kind, String>> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("读取 {} 失败", path.display()))?;
    let mut map = HashMap::new();
    for (i, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = line
            .split_once('=')
            .or_else(|| line.split_once(':'))
            .ok_or_else(|| anyhow::anyhow!("{}:{} 格式应为 key=value", path.display(), i + 1))?;
        let k = k.trim().to_ascii_lowercase();
        let v = v.trim().trim_matches('\'').trim_matches('"').to_string();
        let kind = Kind::parse(&k)
            .ok_or_else(|| anyhow::anyhow!("{}:{} 未知的键: {}", path.display(), i + 1, k))?;
        map.insert(kind, v);
    }
    Ok(map)
}

/// 生成配置文件内容。
pub fn render(entries: &[(Kind, &str)]) -> String {
    let mut s = String::from("\
# JToolchain 项目配置
# 用法：在本目录执行 `jtc use` 即可按此文件一次性切换到指定版本
");
    for (kind, version) in entries {
        s.push_str(&format!("{}={}\n", kind.as_str(), version));
    }
    s
}

/// 读取当前项目的配置；找不到文件时给出明确错误。
pub fn load_or_err() -> Result<(PathBuf, HashMap<Kind, String>)> {
    let cwd = std::env::current_dir().context("无法获取当前目录")?;
    match find(&cwd) {
        Some(p) => {
            let map = read(&p)?;
            if map.is_empty() {
                bail!("{} 里没有配置项", p.display());
            }
            Ok((p, map))
        }
        None => bail!(
            "当前目录未找到 {}（已向上查找到仓库根目录）\n  可用 `jtc init --jdk 21` 生成一份",
            FILE_NAME
        ),
    }
}
