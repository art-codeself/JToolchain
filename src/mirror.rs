//! 下载镜像：把官方下载链接改写到国内镜像。

/// JDK（Adoptium）镜像根。
pub fn jdk_base(mirror: &str) -> Option<&'static str> {
    match mirror {
        "tsinghua" => Some("https://mirrors.tuna.tsinghua.edu.cn/Adoptium"),
        "tencent" => Some("https://mirrors.tuna.tsinghua.edu.cn/Adoptium"),
        _ => None,
    }
}

/// Maven 镜像根（替换 `https://dlcdn.apache.org/maven`）。
pub fn maven_base(mirror: &str) -> Option<&'static str> {
    match mirror {
        "tsinghua" => Some("https://mirrors.tuna.tsinghua.edu.cn/apache/maven"),
        "tencent" => Some("https://mirrors.cloud.tencent.com/apache/maven"),
        "aliyun" => Some("https://mirrors.aliyun.com/apache/maven"),
        "huawei" => Some("https://mirrors.huaweicloud.com/apache/maven"),
        _ => None,
    }
}

/// Gradle 镜像根（替换 `https://services.gradle.org/distributions`）。
pub fn gradle_base(mirror: &str) -> Option<&'static str> {
    match mirror {
        // 清华没有 Gradle 镜像，借用腾讯云的（同样是国内源）
        "tsinghua" | "tencent" => Some("https://mirrors.cloud.tencent.com/gradle"),
        _ => None,
    }
}

pub const OFFICIAL_MAVEN: &str = "https://dlcdn.apache.org/maven";
pub const OFFICIAL_GRADLE: &str = "https://services.gradle.org/distributions";

/// Maven：整体替换路径前缀即可。
pub fn maven_url(url: &str, mirror: &str) -> String {
    match maven_base(mirror) {
        Some(base) => url.replace(OFFICIAL_MAVEN, base).replace(
            "https://archive.apache.org/dist/maven",
            base,
        ),
        None => url.to_string(),
    }
}

/// Gradle：整体替换路径前缀即可。
pub fn gradle_url(url: &str, mirror: &str) -> String {
    match gradle_base(mirror) {
        Some(base) => url.replace(OFFICIAL_GRADLE, base),
        None => url.to_string(),
    }
}

/// JDK：Adoptium 官方是 GitHub release，镜像是「按 major/arch/os 分层」的目录结构，
/// 需要从文件名解析出这几段重新拼装。
///
/// 例：
/// `.../OpenJDK21U-jdk_x64_windows_hotspot_21.0.5_11.zip`
/// -> `https://mirrors.tuna.tsinghua.edu.cn/Adoptium/21/jdk/x64/windows/OpenJDK21U-jdk_x64_windows_hotspot_21.0.5_11.zip`
pub fn jdk_url(url: &str, mirror: &str) -> String {
    let base = match jdk_base(mirror) {
        Some(b) => b,
        None => return url.to_string(),
    };

    let filename = url.rsplit('/').next().unwrap_or_default().to_string();
    if filename.is_empty() {
        return url.to_string();
    }

    // 文件名形如：OpenJDK21U-jdk_x64_windows_hotspot_21.0.5_11.zip
    // 按下划线切分：[OpenJDK21U-jdk, x64, windows, hotspot, 21.0.5, 11.zip]
    let lower = filename.to_lowercase();
    let parts: Vec<&str> = lower.split('_').collect();

    let major = extract_after(&lower, "openjdk")
        .and_then(|s| take_digits(&s))
        .unwrap_or_default();
    let arch = parts.get(1).copied().unwrap_or("").to_string();
    let os = parts.get(2).copied().unwrap_or("").to_string();
    let kind = if lower.contains("-jre") { "jre" } else { "jdk" };

    // 解析不出可信的 arch/os 就退回官方链接，宁可慢也不要 404
    let arch_ok = matches!(arch.as_str(), "x64" | "x86" | "aarch64" | "arm" | "arm64" | "ppc64le" | "s390x");
    let os_ok = matches!(os.as_str(), "windows" | "linux" | "mac" | "aix" | "solaris");
    if major.is_empty() || !arch_ok || !os_ok {
        return url.to_string();
    }

    format!("{}/{}/{}/{}/{}/{}", base, major, kind, arch, os, filename)
}

/// 取 `key` 之后的剩余片段。
fn extract_after(s: &str, key: &str) -> Option<String> {
    s.find(key).map(|i| s[i + key.len()..].to_string())
}

/// 取开头连续数字。
fn take_digits(s: &str) -> Option<String> {
    let d: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    if d.is_empty() {
        None
    } else {
        Some(d)
    }
}
