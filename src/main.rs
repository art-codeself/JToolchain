//! JToolchain CLI (jtc) —— JDK / Maven / Gradle 版本管理器。
//!
//! 核心机制：环境变量里固定指向 `...\.jtc\current\<tool>`，
//! 切换版本只重建 current 的目录链接，因此无需 shell hook，
//! CMD / PowerShell / GitBash / IDE 全局一致，且终端不必重启。

mod cli;
mod cmd;
mod config;
mod env;
mod fsutil;
mod http;
mod mirror;
mod project;
mod tool;
mod ui;

use clap::Parser;

use crate::cli::Cli;

fn main() {
    ui::init();
    let cli = Cli::parse();
    if let Err(e) = cmd::dispatch(cli) {
        ui::err(format!("{:#}", e));
        std::process::exit(1);
    }
}
