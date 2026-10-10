//! 终端输出：统一的标签化日志与交互提示。

use std::io::{self, Write};

use colored::{Color, Colorize};

/// 在 Windows 上开启虚拟终端，使 ANSI 颜色生效。
pub fn init() {
    #[cfg(windows)]
    let _ = colored::control::set_virtual_terminal(true);
}

fn tag(text: &str, color: Color) -> String {
    format!("{}{}{}", "[".dimmed(), text.color(color).bold(), "]".dimmed())
}

pub fn info(msg: impl AsRef<str>) {
    println!("{} {}", tag("info", Color::Cyan), msg.as_ref());
}

pub fn ok(msg: impl AsRef<str>) {
    println!("{} {}", tag(" ok ", Color::Green), msg.as_ref());
}

pub fn warn(msg: impl AsRef<str>) {
    println!("{} {}", tag("warn", Color::Yellow), msg.as_ref());
}

pub fn err(msg: impl AsRef<str>) {
    eprintln!("{} {}", tag("fail", Color::Red), msg.as_ref());
}

pub fn step(msg: impl AsRef<str>) {
    println!("{} {}", tag("....", Color::Blue), msg.as_ref());
}

/// 无装饰的纯文本输出（供脚本消费）。
pub fn plain(msg: impl AsRef<str>) {
    println!("{}", msg.as_ref());
}

pub fn confirm(prompt: &str) -> anyhow::Result<bool> {
    print!("{} {} (y/N) ", tag(" ?  ", Color::Magenta), prompt);
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    let a = buf.trim().to_lowercase();
    Ok(a == "y" || a == "yes")
}

/// 把字节数格式化成人类可读形式。
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut i = 0;
    while size >= 1024.0 && i < UNITS.len() - 1 {
        size /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{:.1} {}", size, UNITS[i])
    }
}
