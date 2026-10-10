//! 全局配置：`$JTC_HOME/settings.json`

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// 可用的下载镜像。
pub const MIRRORS: &[(&str, &str)] = &[
    ("default", "官方源（Adoptium / Apache / Gradle 官方）"),
    ("tsinghua", "清华大学镜像（推荐，国内快）"),
    ("tencent", "腾讯云镜像"),
    ("aliyun", "阿里云镜像"),
    ("huawei", "华为云镜像"),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 下载镜像名，见 `MIRRORS`。
    pub mirror: String,
    /// JDK 发行商：eclipse(temurin) / ibm(semeru) / zulu / oracle / microsoft / openjdk
    pub jdk_vendor: String,
    /// JVM 实现：hotspot / openj9
    pub jdk_impl: String,
    /// 关闭下载进度条（CI 友好）。
    pub no_progress: bool,
    /// 安装后是否自动切换到新版本。
    pub auto_use: bool,
    /// HTTP 超时（秒）。
    pub timeout_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mirror: "tsinghua".into(),
            jdk_vendor: "eclipse".into(),
            jdk_impl: "hotspot".into(),
            no_progress: false,
            auto_use: true,
            timeout_secs: 30,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path)
            .with_context(|| format!("读取配置文件失败: {}", path.display()))?;
        let cfg: Self = serde_json::from_str(&raw)
            .with_context(|| format!("配置文件格式非法: {}", path.display()))?;
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(self)?;
        fs::write(path, raw + "\n")?;
        Ok(())
    }

    pub fn mirror_label(&self) -> &'static str {
        MIRRORS
            .iter()
            .find(|(k, _)| *k == self.mirror)
            .map(|(_, v)| *v)
            .unwrap_or("未知镜像")
    }
}

/// JTC_HOME 下的目录布局。
#[derive(Debug, Clone)]
pub struct Paths {
    pub home: PathBuf,
}

impl Paths {
    /// 优先读环境变量 JTC_HOME，否则落到 ~/.jtc
    pub fn discover() -> Result<Self> {
        let home = match std::env::var_os("JTC_HOME") {
            Some(v) => PathBuf::from(v),
            None => dirs::home_dir()
                .context("无法定位用户主目录，请设置 JTC_HOME 环境变量")?
                .join(".jtc"),
        };
        Ok(Self { home })
    }

    pub fn settings(&self) -> PathBuf {
        self.home.join("settings.json")
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.home.join("cache")
    }

    /// jtc 自身可执行文件所在目录（会被写进 PATH）。
    pub fn bin_dir(&self) -> PathBuf {
        self.home.join("bin")
    }

    /// 某类工具的安装根目录，如 `$JTC_HOME/jdk`
    pub fn tool_dir(&self, kind: &str) -> PathBuf {
        self.home.join(kind)
    }

    /// 某版本目录，如 `$JTC_HOME/jdk/21.0.5+11`
    pub fn version_dir(&self, kind: &str, version: &str) -> PathBuf {
        self.tool_dir(kind).join(version)
    }

    /// current 链接，如 `$JTC_HOME/current/jdk`
    pub fn current_link(&self, kind: &str) -> PathBuf {
        self.home.join("current").join(kind)
    }

    /// 创建基础目录骨架。
    pub fn ensure(&self) -> Result<()> {
        for d in [
            self.home.clone(),
            self.cache_dir(),
            self.bin_dir(),
            self.home.join("current"),
            self.tool_dir("jdk"),
            self.tool_dir("maven"),
            self.tool_dir("gradle"),
        ] {
            fs::create_dir_all(&d)
                .with_context(|| format!("创建目录失败: {}", d.display()))?;
        }
        Ok(())
    }
}
