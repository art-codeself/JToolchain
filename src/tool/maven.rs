//! Maven Provider：Apache 分发目录索引。
//!
//! Apache 的 dlcdn 只保留每个系列的最新版本，历史版本都挪到了 archive.apache.org，
//! 因此这里同时扫描「当前分发」和「归档」两个源，保证 `jtc maven install 3.6.3` 这类也能用。

use anyhow::{bail, Context, Result};
use regex::Regex;

use super::{Kind, Provider, Remote};
use crate::config::Config;
use crate::http;
use crate::mirror;

pub struct Maven;

/// 归档源（只有官方提供，无镜像）。
const ARCHIVE: &str = "https://archive.apache.org/dist/maven";

/// 当前分发源（可走镜像）。
fn current_root(cfg: &Config) -> String {
    match mirror::maven_base(&cfg.mirror) {
        Some(b) => b.to_string(),
        None => mirror::OFFICIAL_MAVEN.to_string(),
    }
}

fn ext() -> &'static str {
    #[cfg(windows)]
    {
        "zip"
    }
    #[cfg(not(windows))]
    {
        "tar.gz"
    }
}

fn download_url(root: &str, version: &str) -> String {
    let major = version.split('.').next().unwrap_or("3").to_string();
    format!(
        "{}/maven-{}/{}/binaries/apache-maven-{}-bin.{}",
        root,
        major,
        version,
        version,
        ext()
    )
}

/// 扫描某个源的目录索引，返回版本号列表。
fn scan_dir(cfg: &Config, root: &str, major: &str) -> Result<Vec<String>> {
    let url = format!("{}/maven-{}/", root, major);
    let html = http::get_text(&url, cfg)?;
    let re = Regex::new(r#"href="(\d+\.\d+(?:\.\d+)?)/""#).context("构建版本正则失败")?;
    let mut vers: Vec<String> = re
        .captures_iter(&html)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
        .collect();
    vers.sort_by(|a, b| super::cmp_version(a, b));
    vers.dedup();
    Ok(vers)
}

/// 合并两个源的版本列表，并给出可用的下载地址。
fn scan_all(cfg: &Config, major: &str) -> Result<Vec<Remote>> {
    let roots = [current_root(cfg), ARCHIVE.to_string()];
    let mut out: Vec<Remote> = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    for root in roots {
        if let Ok(vers) = scan_dir(cfg, &root, major) {
            for v in vers {
                if seen.contains(&v) {
                    continue;
                }
                seen.push(v.clone());
                let archived = root == ARCHIVE;
                out.push(Remote {
                    official: if archived {
                        download_url(ARCHIVE, &v)
                    } else {
                        download_url(mirror::OFFICIAL_MAVEN, &v)
                    },
                    version: v.clone(),
                    url: download_url(&root, &v),
                    note: if archived { "归档".to_string() } else { String::new() },
                });
            }
        }
    }
    Ok(out)
}

impl Provider for Maven {
    fn kind(&self) -> Kind {
        Kind::Maven
    }

    fn list_remote(&self, cfg: &Config) -> Result<Vec<Remote>> {
        let mut out = Vec::new();
        for major in ["3", "4"] {
            out.extend(scan_all(cfg, major)?);
        }
        if out.is_empty() {
            bail!("未能获取 Maven 版本列表，可尝试 `jtc mirror set default` 或 `jtc mirror set tsinghua`");
        }
        out.sort_by(|a, b| super::cmp_version(&a.version, &b.version));
        out.reverse();
        Ok(out)
    }

    fn resolve(&self, req: &str, cfg: &Config) -> Result<Remote> {
        let req = req.trim();
        let root = current_root(cfg);

        // 完整版本号：先试当前分发源，再试归档源
        if Regex::new(r"^\d+\.\d+(\.\d+)?(-[\w.-]+)?$")?.is_match(req) {
            let url = download_url(&root, req);
            if super::url_exists(&url, cfg) {
                return Ok(Remote {
                    version: req.to_string(),
                    official: download_url(mirror::OFFICIAL_MAVEN, req),
                    url: mirror::maven_url(&url, &cfg.mirror),
                    note: String::new(),
                });
            }
            let archived = download_url(ARCHIVE, req);
            if super::url_exists(&archived, cfg) {
                return Ok(Remote {
                    version: req.to_string(),
                    official: archived.clone(),
                    url: archived,
                    note: "归档".into(),
                });
            }
            bail!("Maven {} 不存在，用 `jtc maven ls-remote` 查看可用版本", req);
        }

        // 仅给了主版本：取该系列最新
        let major = req
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>();
        if major.is_empty() {
            bail!("无法识别的 Maven 版本: {}", req);
        }
        let all = scan_all(cfg, &major)?;
        let latest = all
            .iter()
            .filter(|r| r.version.split('.').next() == Some(major.as_str()))
            .max_by(|a, b| super::cmp_version(&a.version, &b.version))
            .ok_or_else(|| anyhow::anyhow!("未找到 Maven {} 系列的可用版本", major))?;
        Ok(latest.clone())
    }
}
