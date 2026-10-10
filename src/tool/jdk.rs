//! JDK Provider：基于 Adoptium（Eclipse Temurin）官方 API。

use anyhow::{bail, Context, Result};
use serde_json::Value;

use super::{Kind, Provider, Remote};
use crate::config::Config;
use crate::http;
use crate::mirror;

const API: &str = "https://api.adoptium.net/v3";

pub struct Jdk;

fn os_name() -> &'static str {
    #[cfg(windows)]
    {
        "windows"
    }
    #[cfg(target_os = "macos")]
    {
        "mac"
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        "linux"
    }
}

fn arch_name() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "aarch64",
        "x86" => "x86",
        "arm" => "arm",
        other => other,
    }
}

fn query(cfg: &Config) -> String {
    format!(
        "architecture={}&os={}&image_type=jdk&vendor={}",
        arch_name(),
        os_name(),
        cfg.jdk_vendor
    )
}

/// 从 API 返回的 version 对象生成目录名，如 `21.0.5+11`。
fn dir_name(v: &Value) -> Option<String> {
    let semver = v["semver"].as_str().unwrap_or_default();
    let build = v["build"].as_u64();
    let core = semver.split('+').next().unwrap_or(semver).to_string();
    if core.is_empty() {
        return None;
    }
    match build {
        Some(b) => Some(format!("{}+{}", core, b)),
        None => Some(core),
    }
}

/// 取资产数组里的第一个可用项。
fn first_asset(arr: &Value) -> Result<&Value> {
    arr.as_array()
        .and_then(|a| a.first())
        .ok_or_else(|| anyhow::anyhow!("Adoptium 未返回该版本的构建产物"))
}

fn to_remote(asset: &Value, cfg: &Config) -> Result<Remote> {
    let pkg = &asset["binary"]["package"];
    let url = pkg["link"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("响应缺少下载链接"))?
        .to_string();
    let version_obj = if asset["version"].is_object() {
        &asset["version"]
    } else {
        &asset["version_data"]
    };
    let version = dir_name(version_obj)
        .or_else(|| asset["release_name"].as_str().map(|s| s.trim_start_matches("jdk-").to_string()))
        .context("无法解析版本号")?;

    let size = pkg["size"].as_u64().unwrap_or(0);
    let official = url.clone();
    let url = mirror::jdk_url(&url, &cfg.mirror);
    let note = format!("{} {} · {}", cfg.jdk_vendor, cfg.jdk_impl, crate::ui::human_size(size));
    Ok(Remote {
        version,
        url,
        official,
        note,
    })
}

impl Provider for Jdk {
    fn kind(&self) -> Kind {
        Kind::Jdk
    }

    fn list_remote(&self, cfg: &Config) -> Result<Vec<Remote>> {
        let url = format!("{}/info/available_releases", API);
        let v: Value = http::get_json(&url, cfg)?;
        let lts: Vec<u64> = v["available_lts_releases"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_u64()).collect())
            .unwrap_or_default();
        let releases: Vec<u64> = v["available_releases"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_u64()).collect())
            .unwrap_or_default();

        let list = if releases.is_empty() { lts.clone() } else { releases };
        if list.is_empty() {
            bail!("未能获取可用 JDK 版本列表（网络不可达或镜像异常）");
        }

        let mut out: Vec<Remote> = list
            .iter()
            .map(|n| {
                let is_lts = lts.contains(n);
                Remote {
                    version: n.to_string(),
                    url: String::new(),
                    official: String::new(),
                    note: if is_lts { "LTS".into() } else { String::new() },
                }
            })
            .collect();
        out.sort_by(|a, b| {
            b.version
                .parse::<u64>()
                .unwrap_or(0)
                .cmp(&a.version.parse::<u64>().unwrap_or(0))
        });
        Ok(out)
    }

    fn resolve(&self, req: &str, cfg: &Config) -> Result<Remote> {
        let req = req.trim();
        let is_feature = !req.contains('.') && req.chars().all(|c| c.is_ascii_digit());

        if is_feature {
            let url = format!(
                "{}/assets/latest/{}/{}?{}",
                API,
                req,
                cfg.jdk_impl,
                query(cfg)
            );
            let arr: Value = http::get_json(&url, cfg)?;
            return to_remote(first_asset(&arr)?, cfg);
        }

        // 形如 21.0.5 或 21.0.5+11
        let encoded = req.replace('+', "%2B");
        let url = format!(
            "{}/assets/version/{}?{}",
            API,
            encoded,
            query(cfg)
        );
        if let Ok(arr) = http::get_json::<Value>(&url, cfg) {
            if arr.as_array().map(|a| !a.is_empty()).unwrap_or(false) {
                return to_remote(first_asset(&arr)?, cfg);
            }
        }

        // 回退：按主版本拉最新
        let major: String = req.chars().take_while(|c| c.is_ascii_digit()).collect();
        if major.is_empty() {
            bail!("无法识别的 JDK 版本: {}", req);
        }
        let url = format!(
            "{}/assets/latest/{}/{}?{}",
            API,
            major,
            cfg.jdk_impl,
            query(cfg)
        );
        let arr: Value = http::get_json(&url, cfg)?;
        to_remote(first_asset(&arr)?, cfg)
    }
}
