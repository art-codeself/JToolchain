//! Gradle Provider：services.gradle.org 版本清单。

use anyhow::{bail, Result};
use serde_json::Value;

use super::{Kind, Provider, Remote};
use crate::config::Config;
use crate::http;
use crate::mirror;

const VERSIONS: &str = "https://services.gradle.org/versions/all";
const DIST: &str = "https://services.gradle.org/distributions";

pub struct Gradle;

fn dist_url(version: &str) -> String {
    format!("{}/gradle-{}-bin.zip", DIST, version)
}

/// 稳定版过滤：排除 snapshot / nightly / rc / milestone / broken。
fn is_stable(v: &Value) -> bool {
    let flag = |k: &str| v[k].as_bool().unwrap_or(false);
    if flag("snapshot") || flag("nightly") || flag("activeRc") || flag("broken") {
        return false;
    }
    let empty = |k: &str| v[k].as_str().unwrap_or_default().is_empty();
    !(!empty("rcFor") || !empty("milestoneFor"))
}

fn fetch_all(cfg: &Config) -> Result<Vec<Value>> {
    let arr: Value = http::get_json(VERSIONS, cfg)?;
    let list: Vec<Value> = arr
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(is_stable)
        .collect();
    if list.is_empty() {
        bail!("未能获取 Gradle 版本列表，可尝试 `jtc mirror set default`（该接口无镜像）");
    }
    Ok(list)
}

impl Provider for Gradle {
    fn kind(&self) -> Kind {
        Kind::Gradle
    }

    fn list_remote(&self, cfg: &Config) -> Result<Vec<Remote>> {
        let mut out: Vec<Remote> = fetch_all(cfg)?
            .iter()
            .filter_map(|v| {
                let version = v["version"].as_str()?.to_string();
                Some(Remote {
                    version: version.clone(),
                    url: String::new(),
                    official: dist_url(&version),
                    note: String::new(),
                })
            })
            .collect();
        out.sort_by(|a, b| super::cmp_version(&a.version, &b.version));
        out.reverse();
        Ok(out)
    }

    fn resolve(&self, req: &str, cfg: &Config) -> Result<Remote> {
        let req = req.trim();
        let list = fetch_all(cfg)?;
        let versions: Vec<String> = list
            .iter()
            .filter_map(|v| v["version"].as_str().map(|s| s.to_string()))
            .collect();

        // 精确匹配
        let hit = versions.iter().find(|v| v.as_str() == req).cloned().or_else(|| {
            // 前缀匹配：8 -> 8.x 最高版本
            versions
                .iter()
                .filter(|v| {
                    v.starts_with(req)
                        && (v.len() == req.len()
                            || matches!(v.as_bytes().get(req.len()).copied(), Some(b'.') | Some(b'-')))
                })
                .max_by(|a, b| super::cmp_version(a, b))
                .cloned()
        });

        match hit {
            Some(v) => {
                let official = dist_url(&v);
                let url = mirror::gradle_url(&official, &cfg.mirror);
                Ok(Remote {
                    version: v,
                    url,
                    official,
                    note: String::new(),
                })
            }
            None => bail!("未找到 Gradle 版本: {}", req),
        }
    }
}
