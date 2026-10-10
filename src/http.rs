//! HTTP：JSON 拉取与带进度条的文件下载。

use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use serde::de::DeserializeOwned;

use crate::config::Config;

fn agent(cfg: &Config) -> ureq::Agent {
    let mut builder = ureq::AgentBuilder::new()
        .try_proxy_from_env(true)
        .timeout_connect(Duration::from_secs(cfg.timeout_secs))
        .timeout_read(Duration::from_secs(cfg.timeout_secs * 6))
        .user_agent(concat!("jtc/", env!("CARGO_PKG_VERSION")));
    builder = builder.redirects(5);
    builder.build()
}

pub fn get_text(url: &str, cfg: &Config) -> Result<String> {
    let resp = agent(cfg).get(url).call().map_err(|e| map_err(url, e))?;
    let status = resp.status();
    if status != 200 {
        bail!("请求 {} 返回 HTTP {}", url, status);
    }
    resp.into_string()
        .with_context(|| format!("读取响应失败: {}", url))
}

pub fn get_json<T: DeserializeOwned>(url: &str, cfg: &Config) -> Result<T> {
    let text = get_text(url, cfg)?;
    serde_json::from_str(&text).with_context(|| format!("解析 JSON 失败: {}", url))
}

/// 流式下载到文件，返回字节数。
pub fn download(url: &str, dest: &Path, cfg: &Config) -> Result<u64> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    let resp = agent(cfg).get(url).call().map_err(|e| map_err(url, e))?;
    let status = resp.status();
    if status != 200 {
        bail!("下载 {} 返回 HTTP {}", url, status);
    }

    let total: u64 = resp
        .header("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let pb = if cfg.no_progress {
        ProgressBar::hidden()
    } else {
        let pb = ProgressBar::new(total);
        pb.set_style(
            ProgressStyle::with_template(
                "{spinner:.cyan} [{bar:32.cyan/blue}] {bytes}/{total_bytes} · {bytes_per_sec} · {eta}",
            )
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("=>-"),
        );
        pb
    };

    let mut reader = ProgressReader {
        inner: resp.into_reader(),
        pb: pb.clone(),
    };

    let tmp = dest.with_extension("part");
    let mut written: u64 = 0;
    {
        let mut out = fs::File::create(&tmp)?;
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            written += n as u64;
        }
        out.flush()?;
    }
    fs::rename(&tmp, dest)?;
    pb.finish_and_clear();
    Ok(written)
}

struct ProgressReader<R: Read> {
    inner: R,
    pb: ProgressBar,
}

impl<R: Read> Read for ProgressReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.pb.inc(n as u64);
        }
        Ok(n)
    }
}

fn map_err(url: &str, e: ureq::Error) -> anyhow::Error {
    match e {
        ureq::Error::Status(code, _) => anyhow::anyhow!("请求 {} 失败，HTTP {}", url, code),
        ureq::Error::Transport(t) => anyhow::anyhow!(
            "网络请求失败: {}\n  URL: {}\n  提示：可尝试 `jtc mirror set tsinghua` 切换国内镜像，或检查代理设置",
            t,
            url
        ),
    }
}
