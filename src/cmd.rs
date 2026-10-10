//! 命令实现。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcCommand;

use anyhow::{bail, Context, Result};
use colored::Colorize;

use crate::cli::{Cli, Command, MirrorAction, ToolAction};
use crate::config::{Config, Paths, MIRRORS};
use crate::fsutil;
use crate::http;
use crate::project;
use crate::tool::{self, Kind};
use crate::{env, ui};

pub struct Ctx {
    pub paths: Paths,
    pub cfg: Config,
    pub cfg_path: PathBuf,
}

impl Ctx {
    fn save(&self) -> Result<()> {
        self.cfg.save(&self.cfg_path)
    }

    /// 首次使用时提醒先跑 setup。
    fn ensure_ready(&self) {
        if !self.paths.home.exists() {
            ui::warn("尚未初始化，建议先执行 `jtc setup`");
        }
    }
}

pub fn dispatch(cli: Cli) -> Result<()> {
    let paths = Paths::discover()?;
    let cfg_path = paths.settings();
    let cfg = Config::load(&cfg_path)?;

    // 除 setup 外，其余命令都需要目录骨架存在
    if !matches!(cli.command, Command::Setup { .. }) {
        paths.ensure()?;
    }

    let ctx = Ctx { paths, cfg, cfg_path };

    match cli.command {
        Command::Setup { force, system } => cmd_setup(&ctx, force, system),
        Command::Ls { kind, remote } => cmd_ls(&ctx, kind, remote),
        Command::Use { kind, version } => cmd_use(&ctx, kind, version),
        Command::Current { kind } => cmd_current(&ctx, kind),
        Command::Which { exe } => cmd_which(&ctx, exe),
        Command::Env { shell } => cmd_env(&ctx, &shell),
        Command::Init { jdk, maven, gradle } => cmd_init(&ctx, jdk, maven, gradle),
        Command::Doctor => cmd_doctor(&ctx),
        Command::Mirror { action } => cmd_mirror(&ctx, action),
        Command::Jdk(t) => cmd_tool(&ctx, Kind::Jdk, t.action),
        Command::Maven(t) => cmd_tool(&ctx, Kind::Maven, t.action),
        Command::Gradle(t) => cmd_tool(&ctx, Kind::Gradle, t.action),
    }
}

// ---------------- setup ----------------

fn cmd_setup(ctx: &Ctx, force: bool, system: bool) -> Result<()> {
    let paths = &ctx.paths;
    paths.ensure()?;
    ui::info(format!("JTC_HOME = {}", paths.home.display()));

    if force || !ctx.cfg_path.exists() {
        ctx.save()?;
    }

    // 把自身复制到 $JTC_HOME/bin，这样 PATH 只需配置一次
    let exe = std::env::current_exe().context("无法定位 jtc 自身路径")?;
    let dest = paths.bin_dir().join(if cfg!(windows) { "jtc.exe" } else { "jtc" });
    if exe != dest {
        match std::fs::copy(&exe, &dest) {
            Ok(_) => ui::step(format!("已安装 jtc 到 {}", dest.display())),
            Err(e) => ui::warn(format!(
                "复制 jtc 到 {} 失败: {}（若该文件正在使用，可稍后手动复制）",
                dest.display(),
                e
            )),
        }
    }

    #[cfg(windows)]
    {
        let home = paths.home.to_string_lossy().to_string();
        let home = home.trim_end_matches(['\\', '/']).to_string();
        env::set_expand("JTC_HOME", &home)?;
        // 同样写绝对路径：mvn.cmd / gradle.bat / IDE 读 JAVA_HOME 时未必会展开 %JTC_HOME%
        env::set_expand("JAVA_HOME", &format!("{}\\current\\jdk", home))?;
        env::set_expand("MAVEN_HOME", &format!("{}\\current\\maven", home))?;
        env::set_expand("M2_HOME", &format!("{}\\current\\maven", home))?;
        env::set_expand("GRADLE_HOME", &format!("{}\\current\\gradle", home))?;

        let changed = env::ensure_path_entries(&home)?;
        env::broadcast();

        let promoted = if system {
            match env::promote_to_machine_path(&home) {
                Ok(true) => {
                    ui::ok("已把 jtc 提升到系统 PATH 最前（压过 JDK/Oracle 自带条目）");
                    true
                }
                Ok(false) => {
                    ui::step("系统 PATH 已包含所需排序，无需改动");
                    false
                }
                Err(e) => {
                    ui::warn(format!("写入系统 PATH 失败: {:#}", e));
                    ui::info("请「以管理员身份」重新打开终端，再执行一次 `jtc setup --system`");
                    false
                }
            }
        } else {
            false
        };

        if changed {
            ui::ok("已把 jtc / java / mvn / gradle 写入用户 PATH");
        } else if !promoted {
            ui::step("PATH 已包含所需条目，无需改动");
        }
        ui::ok("环境变量写入完成（JAVA_HOME / MAVEN_HOME / GRADLE_HOME）");
        ui::info("新开的终端立即生效；已打开的终端里 JAVA_HOME 指向固定路径，也会自动跟随切换");
        ui::info("验证方式：重新打开终端后执行 `jtc doctor`");
    }

    #[cfg(not(windows))]
    {
        let sh = paths.home.join("env.sh");
        fsutil::write_string(&sh, &env::shell_exports(&paths.home, "bash"))?;
        ui::ok(format!("已生成 {}", sh.display()));
        ui::info("请把下面这行加入 ~/.bashrc 或 ~/.zshrc：");
        ui::plain(format!("  source \"{}\"", sh.display()));
    }

    Ok(())
}

// ---------------- ls ----------------

fn cmd_ls(ctx: &Ctx, kind: Option<String>, remote: bool) -> Result<()> {
    match kind {
        Some(k) => {
            let kind = parse_kind(&k)?;
            cmd_tool(ctx, kind, ToolAction::Ls { remote })
        }
        None => {
            for kind in Kind::all() {
                print_installed(ctx, kind)?;
            }
            Ok(())
        }
    }
}

fn print_installed(ctx: &Ctx, kind: Kind) -> Result<()> {
    let vers = tool::installed(&ctx.paths, kind);
    let cur = tool::current(&ctx.paths, kind);
    ui::plain("");
    ui::plain(format!("{}  (共 {} 个已安装)", kind.label(), vers.len()));
    if vers.is_empty() {
        ui::plain(format!(
            "    暂无，安装：`jtc {} install {}`",
            kind.as_str(),
            sample_version(kind)
        ));
        return Ok(());
    }
    for v in &vers {
        if cur.as_deref() == Some(v.as_str()) {
            ui::plain(format!("  * {:<16} <- current", v));
        } else {
            ui::plain(format!("    {:<16}", v));
        }
    }
    Ok(())
}

fn sample_version(kind: Kind) -> &'static str {
    match kind {
        Kind::Jdk => "21",
        Kind::Maven => "3.9.6",
        Kind::Gradle => "8.5",
    }
}

fn print_remote(ctx: &Ctx, kind: Kind) -> Result<()> {
    let p = tool::provider(kind);
    ui::step(format!("正在获取可用的 {} 版本...", kind.label()));
    let list = p.list_remote(&ctx.cfg)?;
    let cur = tool::current(&ctx.paths, kind);
    ui::plain("");
    for r in list.iter().take(40) {
        let mark = if cur.as_deref() == Some(r.version.as_str()) {
            " *"
        } else {
            ""
        };
        if r.note.is_empty() {
            ui::plain(format!("  {}{}", r.version, mark));
        } else {
            ui::plain(format!("  {:<20} {}{}", r.version, r.note, mark));
        }
    }
    if list.len() > 40 {
        ui::plain(format!("  ... 另有 {} 个版本未显示", list.len() - 40));
    }
    ui::plain("");
    ui::info(format!(
        "安装：`jtc {} install <版本>`（当前镜像：{}）",
        kind.as_str(),
        ctx.cfg.mirror_label()
    ));
    Ok(())
}

// ---------------- use / current ----------------

fn cmd_use(ctx: &Ctx, kind: Option<String>, version: Option<String>) -> Result<()> {
    match (kind, version) {
        (Some(k), Some(v)) => {
            let kind = parse_kind(&k)?;
            do_use(ctx, kind, &v)
        }
        (Some(k), None) => {
            let kind = parse_kind(&k)?;
            let (file, map) = project::load_or_err()?;
            let v = map
                .get(&kind)
                .ok_or_else(|| anyhow::anyhow!("{} 里没有配置 {}", file.display(), kind.as_str()))?;
            do_use(ctx, kind, v)
        }
        (None, Some(_)) => bail!("请指定工具类型，例如 `jtc use jdk 21`"),
        (None, None) => {
            let (file, map) = project::load_or_err()?;
            ui::info(format!("按 {} 切换", file.display()));
            for kind in [Kind::Jdk, Kind::Maven, Kind::Gradle] {
                if let Some(v) = map.get(&kind) {
                    do_use(ctx, kind, v)?;
                }
            }
            Ok(())
        }
    }
}

fn do_use(ctx: &Ctx, kind: Kind, version: &str) -> Result<()> {
    let v = tool::match_installed(&ctx.paths, kind, version)?;
    tool::use_version(&ctx.paths, kind, &v)?;
    ui::ok(format!(
        "{} -> {}   ({} = {})",
        kind.label(),
        v,
        kind.home_var(),
        ctx.paths.current_link(kind.as_str()).display()
    ));
    Ok(())
}

fn cmd_current(ctx: &Ctx, kind: Option<String>) -> Result<()> {
    match kind {
        Some(k) => {
            let kind = parse_kind(&k)?;
            print_current(ctx, kind)
        }
        None => {
            for kind in Kind::all() {
                print_current(ctx, kind)?;
            }
            Ok(())
        }
    }
}

fn print_current(ctx: &Ctx, kind: Kind) -> Result<()> {
    match tool::current(&ctx.paths, kind) {
        Some(v) => {
            let probe = probe_version(ctx, kind).unwrap_or_default();
            if probe.is_empty() {
                ui::plain(format!("{}: {}", kind.label(), v));
            } else {
                ui::plain(format!("{}: {}  ({})", kind.label(), v, probe));
            }
        }
        None => ui::plain(format!("{}: 未设置", kind.label())),
    }
    Ok(())
}

/// 实际执行 `java -version` / `mvn -version` 等，取首行作为展示。
fn probe_version(ctx: &Ctx, kind: Kind) -> Option<String> {
    let exe_name = match kind {
        Kind::Jdk => "java",
        Kind::Maven => "mvn",
        Kind::Gradle => "gradle",
    };
    let path = tool::current_exe(&ctx.paths, kind, exe_name)?;

    #[cfg(windows)]
    let output = {
        let lower = path.to_string_lossy().to_lowercase();
        if lower.ends_with(".cmd") || lower.ends_with(".bat") {
            ProcCommand::new("cmd")
                .arg("/c")
                .arg(path.as_os_str())
                .arg("-version")
                .output()
        } else {
            ProcCommand::new(&path).arg("-version").output()
        }
    };
    #[cfg(not(windows))]
    let output = ProcCommand::new(&path).arg("-version").output();

    let out = output.ok()?;
    let mut s = String::from_utf8_lossy(&out.stdout).to_string();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    s.lines().find(|l| !l.trim().is_empty()).map(|l| l.trim().to_string())
}

// ---------------- which / env ----------------

fn cmd_which(ctx: &Ctx, exe: Option<String>) -> Result<()> {
    match exe {
        Some(name) => {
            let (kind, path) = tool::find_any(&ctx.paths, &name).ok_or_else(|| {
                anyhow::anyhow!(
                    "未找到 {}（请确认已安装并 `jtc <tool> use <version>`）",
                    name
                )
            })?;
            ui::plain(path.display().to_string());
            let _ = kind;
            Ok(())
        }
        None => {
            for (kind, exe) in [
                (Kind::Jdk, "java"),
                (Kind::Jdk, "javac"),
                (Kind::Maven, "mvn"),
                (Kind::Gradle, "gradle"),
            ] {
                match tool::current_exe(&ctx.paths, kind, exe) {
                    Some(p) => ui::plain(format!("{:<8} {}", exe, p.display())),
                    None => ui::plain(format!("{:<8} <未安装>", exe)),
                }
            }
            Ok(())
        }
    }
}

fn cmd_env(ctx: &Ctx, shell: &str) -> Result<()> {
    let shell = if shell == "auto" {
        if cfg!(windows) {
            "powershell"
        } else {
            "bash"
        }
    } else {
        shell
    };
    ui::plain(env::shell_exports(&ctx.paths.home, shell).trim_end().to_string());
    Ok(())
}

// ---------------- init / doctor ----------------

fn cmd_init(
    ctx: &Ctx,
    jdk: Option<String>,
    maven: Option<String>,
    gradle: Option<String>,
) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let file = cwd.join(project::FILE_NAME);
    if file.exists() && !ui::confirm("已存在 .jtoolchain，是否覆盖")? {
        return Ok(());
    }

    let mut entries: Vec<(Kind, &str)> = Vec::new();
    let jdk_s;
    let maven_s;
    let gradle_s;
    if let Some(v) = jdk.as_deref() {
        entries.push((Kind::Jdk, v));
    } else if let Some(c) = tool::current(&ctx.paths, Kind::Jdk) {
        jdk_s = c;
        entries.push((Kind::Jdk, &jdk_s));
    }
    if let Some(v) = maven.as_deref() {
        entries.push((Kind::Maven, v));
    } else if let Some(c) = tool::current(&ctx.paths, Kind::Maven) {
        maven_s = c;
        entries.push((Kind::Maven, &maven_s));
    }
    if let Some(v) = gradle.as_deref() {
        entries.push((Kind::Gradle, v));
    } else if let Some(c) = tool::current(&ctx.paths, Kind::Gradle) {
        gradle_s = c;
        entries.push((Kind::Gradle, &gradle_s));
    }

    if entries.is_empty() {
        bail!("没有可写入的版本，请用 --jdk / --maven / --gradle 指定");
    }

    fsutil::write_string(&file, &project::render(&entries))?;
    ui::ok(format!("已生成 {}", file.display()));
    for (k, v) in &entries {
        ui::plain(format!("  {}={}", k.as_str(), v));
    }
    ui::info("把该文件提交到仓库，团队成员执行 `jtc use` 即可对齐工具链");
    Ok(())
}

fn cmd_doctor(ctx: &Ctx) -> Result<()> {
    let home = &ctx.paths.home;
    ui::plain(format!("JTC_HOME = {}", home.display()));
    check(home.exists(), "JTC_HOME 目录存在");
    check(
        ctx.paths.bin_dir().exists(),
        &format!("bin 目录存在 ({})", ctx.paths.bin_dir().display()),
    );

    for kind in Kind::all() {
        let link = ctx.paths.current_link(kind.as_str());
        let cur = tool::current(&ctx.paths, kind);
        match cur {
            Some(v) => {
                let dir = ctx.paths.version_dir(kind.as_str(), &v);
                check(
                    dir.is_dir(),
                    &format!("{} current -> {}（{}）", kind.label(), v, dir.display()),
                );
            }
            None => check(false, &format!("{} current 未设置", kind.label())),
        }
        let _ = link;
    }

    // PATH 检查
    let sys_path = std::env::var("PATH").unwrap_or_default();
    let want: Vec<PathBuf> = vec![
        ctx.paths.bin_dir(),
        ctx.paths.current_link("jdk").join("bin"),
        ctx.paths.current_link("maven").join("bin"),
        ctx.paths.current_link("gradle").join("bin"),
    ];
    // 展开环境变量 + 兼容 Git Bash 的 /c/... 形式，否则在这类 shell 里会误报缺失。
    // 注意分隔符不能把 `:` 一并盲切——Windows PATH 里 `C:\...` 的盘符冒号会被切断。
    // Git Bash/MSYS 的 PATH 是 POSIX 形式（以 `/` 开头、`:` 分隔），据此区分两种风格。
    let norm = |p: &PathBuf| env::normalize_path(&p.to_string_lossy());
    let posix_style = sys_path.trim_start().starts_with('/');
    let has: Vec<String> = sys_path
        .split(if posix_style { ':' } else { ';' })
        .map(|s| s.to_string())
        .collect();
    for w in want {
        let hit = has.iter().any(|h| norm(&PathBuf::from(h.as_str())) == norm(&w));
        check(hit, &format!("PATH 含 {}", w.display()));
    }

    // 真正决定 `java -version` 结果的是「谁排得更前」，这一步定位切了却不生效的根因
    shadow_check(ctx);

    for (name, expect) in [
        ("JAVA_HOME", ctx.paths.current_link("jdk")),
        ("MAVEN_HOME", ctx.paths.current_link("maven")),
        ("GRADLE_HOME", ctx.paths.current_link("gradle")),
    ] {
        let actual = std::env::var(name).unwrap_or_default();
        check(
            !actual.is_empty() && norm(&PathBuf::from(&actual)) == norm(&expect),
            &format!("{} = {}", name, if actual.is_empty() { "<未设置>".into() } else { actual }),
        );
    }

    for (kind, exe) in [
        (Kind::Jdk, "java"),
        (Kind::Maven, "mvn"),
        (Kind::Gradle, "gradle"),
    ] {
        let p = tool::current_exe(&ctx.paths, kind, exe);
        check(p.is_some(), &format!("{} 可执行文件可用", exe));
    }

    ui::plain("");
    ui::info(format!("镜像：{}（{}）", ctx.cfg.mirror, ctx.cfg.mirror_label()));
    Ok(())
}

fn check(ok: bool, label: &str) {
    if ok {
        ui::plain(format!("  [{}] {}", "OK".green(), label));
    } else {
        ui::plain(format!("  [{}] {}", "!!".red(), label.red()));
    }
}

/// 沿真实 PATH 顺序找出每个命令的第一个命中项，看是否被别的安装目录抢走。
fn shadow_check(ctx: &Ctx) {
    let full = env::effective_path();
    let posix = full.trim_start().starts_with('/');
    let items: Vec<String> = full
        .split(if posix { ':' } else { ';' })
        .map(|s| s.to_string())
        .collect();

    for (kind, exe) in [
        (Kind::Jdk, "java"),
        (Kind::Maven, "mvn"),
        (Kind::Gradle, "gradle"),
    ] {
        let want = ctx.paths.current_link(kind.as_str()).join("bin");
        let want_norm = env::normalize_path(&want.to_string_lossy());
        let mut hit = false;
        for item in &items {
            let dir = PathBuf::from(env::expand_path(item));
            let found = tool::exe_candidates(exe).iter().any(|c| dir.join(c).exists());
            if !found {
                continue;
            }
            if env::normalize_path(item) == want_norm {
                check(true, &format!("{} 优先命中 {}", exe, want.display()));
            } else {
                check(
                    false,
                    &format!("{} 实际用的还是 {}（它在 PATH 里排在 jtc 之前）", exe, dir.display()),
                );
                ui::info("修复：`jtc setup --system`（需管理员）可把 jtc 提到系统 PATH 最前");
            }
            hit = true;
            break;
        }
        if !hit {
            check(false, &format!("PATH 里找不到 {}", exe));
        }
    }
}

// ---------------- mirror ----------------

fn cmd_mirror(ctx: &Ctx, action: Option<MirrorAction>) -> Result<()> {
    let mut cfg = ctx.cfg.clone();
    match action.unwrap_or(MirrorAction::Show) {
        MirrorAction::Show => {
            ui::plain(format!("{}  ({})", cfg.mirror, cfg.mirror_label()));
        }
        MirrorAction::List => {
            for (k, v) in MIRRORS {
                let mark = if *k == cfg.mirror { " *" } else { "" };
                ui::plain(format!("  {:<10} {}{}", k, v, mark));
            }
        }
        MirrorAction::Set { name } => {
            if !MIRRORS.iter().any(|(k, _)| *k == name) {
                bail!(
                    "未知镜像 {}，可选：{}",
                    name,
                    MIRRORS.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(", ")
                );
            }
            cfg.mirror = name.clone();
            cfg.save(&ctx.cfg_path)?;
            ui::ok(format!("镜像已切换为 {}（{}）", name, cfg.mirror_label()));
        }
    }
    Ok(())
}

// ---------------- 工具子命令分发 ----------------

fn cmd_tool(ctx: &Ctx, kind: Kind, action: ToolAction) -> Result<()> {
    match action {
        ToolAction::Ls { remote } => {
            if remote {
                print_remote(ctx, kind)
            } else {
                print_installed(ctx, kind)
            }
        }
        ToolAction::LsRemote => print_remote(ctx, kind),
        ToolAction::Install { version, no_use } => cmd_install(ctx, kind, &version, no_use),
        ToolAction::Use { version } => match version {
            Some(v) => do_use(ctx, kind, &v),
            None => {
                let (file, map) = project::load_or_err()?;
                let v = map
                    .get(&kind)
                    .ok_or_else(|| anyhow::anyhow!("{} 里没有配置 {}", file.display(), kind.as_str()))?
                    .clone();
                do_use(ctx, kind, &v)
            }
        },
        ToolAction::Uninstall { version, yes } => {
            let v = tool::match_installed(&ctx.paths, kind, &version)?;
            if !yes
                && !ui::confirm(&format!("确认卸载 {} {}？这会删除整个目录", kind.label(), v))?
            {
                ui::info("已取消");
                return Ok(());
            }
            tool::uninstall(&ctx.paths, kind, &v)?;
            ui::ok(format!("已卸载 {} {}", kind.label(), v));
            Ok(())
        }
        ToolAction::Link { version, path } => cmd_link(ctx, kind, &version, &path),
        ToolAction::Scan { link } => cmd_scan(ctx, kind, link),
        ToolAction::Current => print_current(ctx, kind),
        ToolAction::Home => {
            let link = ctx.paths.current_link(kind.as_str());
            if !link.exists() {
                bail!("{} 尚未设置当前版本", kind.label());
            }
            ui::plain(link.display().to_string());
            Ok(())
        }
    }
}

/// 登记本机已有的安装目录：只建链接，不复制不移动。
fn cmd_link(ctx: &Ctx, kind: Kind, version: &str, path: &str) -> Result<()> {
    let target = PathBuf::from(path);
    let target = if target.is_absolute() {
        target
    } else {
        std::env::current_dir()?.join(&target)
    };

    tool::validate_dir(kind, &target)?;

    let dest = ctx.paths.version_dir(kind.as_str(), version);
    if dest.exists() || fsutil::is_link(&dest) {
        bail!(
            "版本 {} 已存在（{}）\n  先执行 `jtc {} uninstall {}` 再登记",
            version,
            dest.display(),
            kind.as_str(),
            version
        );
    }

    fsutil::link_dir(&target, &dest)?;
    ui::ok(format!("已登记 {} {}", kind.label(), version));
    ui::plain(format!("  {} -> {}", dest.display(), target.display()));

    if ctx.cfg.auto_use && tool::current(&ctx.paths, kind).is_none() {
        tool::use_version(&ctx.paths, kind, version)?;
        ui::ok("当前无生效版本，已自动切换过来");
    } else {
        ui::info(format!("切换：`jtc {} use {}", kind.as_str(), version));
    }
    Ok(())
}

/// 扫描常见安装位置。
fn cmd_scan(ctx: &Ctx, kind: Kind, do_link: bool) -> Result<()> {
    ui::step(format!("扫描本机已安装的 {} ...", kind.label()));
    let home_lower = ctx.paths.home.to_string_lossy().to_lowercase();
    let registered: Vec<String> = tool::installed(&ctx.paths, kind);

    let mut found: Vec<(String, PathBuf)> = Vec::new();
    for root in tool::scan_roots(kind) {
        collect(&mut found, &root, &home_lower, kind);
        if let Ok(rd) = fs::read_dir(&root) {
            for entry in rd.filter_map(|e| e.ok()) {
                let p = entry.path();
                if p.is_dir() {
                    collect(&mut found, &p, &home_lower, kind);
                }
            }
        }
    }

    if found.is_empty() {
        ui::warn(format!(
            "没有扫描到 {}。若装在自定义位置，用 `jtc {} link <版本> <路径>` 手动登记",
            kind.label(),
            kind.as_str()
        ));
        return Ok(());
    }

    ui::plain("");
    ui::plain(format!("发现 {} 个 {}：", found.len(), kind.label()));
    for (v, p) in &found {
        let state = if registered.contains(v) {
            "（已登记）".dimmed().to_string()
        } else {
            String::new()
        };
        ui::plain(format!("  {:<16} {} {}", v, p.display(), state));
    }
    ui::plain("");

    if !do_link {
        ui::info(format!(
            "登记：`jtc {} link <版本> <路径>`，或加 `--link` 一次性全部登记",
            kind.as_str()
        ));
        return Ok(());
    }

    let mut n = 0;
    for (v, p) in &found {
        if registered.contains(v) {
            continue;
        }
        match cmd_link(ctx, kind, v, &p.to_string_lossy()) {
            Ok(()) => n += 1,
            Err(e) => ui::warn(format!("跳过 {}: {}", v, e)),
        }
    }
    if n > 0 {
        ui::ok(format!("本次登记 {} 个 {}", n, kind.label()));
    }
    Ok(())
}

fn collect(found: &mut Vec<(String, PathBuf)>, dir: &Path, home_lower: &str, kind: Kind) {
    let s = dir.to_string_lossy().to_lowercase();
    // 跳过 jtc 自己的安装目录，避免自己登记自己
    if s.starts_with(home_lower) {
        return;
    }
    if tool::validate_dir(kind, dir).is_err() {
        return;
    }
    if found.iter().any(|(_, p)| p == dir) {
        return;
    }
    let name = dir.file_name().unwrap_or_default().to_string_lossy();
    let v = tool::version_from_name(&name).unwrap_or_else(|| name.to_string());
    found.push((v, dir.to_path_buf()));
}

fn cmd_install(ctx: &Ctx, kind: Kind, version: &str, no_use: bool) -> Result<()> {
    ctx.ensure_ready();
    let p = tool::provider(kind);
    ui::step(format!("解析 {} {} ...", kind.label(), version));
    let remote = p.resolve(version, &ctx.cfg)?;

    let dest = ctx.paths.version_dir(kind.as_str(), &remote.version);
    if dest.is_dir() {
        ui::info(format!("{} {} 已安装，跳过下载", kind.label(), remote.version));
        return finish_install(ctx, kind, &remote.version, no_use);
    }

    let ext = if remote.url.contains(".tar.gz") {
        "tar.gz"
    } else if remote.url.contains(".tgz") {
        "tgz"
    } else {
        "zip"
    };
    let cache = ctx
        .paths
        .cache_dir()
        .join(format!("{}-{}.{}", kind.as_str(), remote.version, ext));

    // 镜像可能尚未同步或不提供该版本，下载前先探一次，失效就自动回退官方源
    let mut dl_url = remote.url.clone();
    if !remote.official.is_empty()
        && dl_url != remote.official
        && !tool::url_exists(&dl_url, &ctx.cfg)
    {
        ui::warn(format!(
            "镜像 {} 上没有该版本，自动回退官方源",
            ctx.cfg.mirror
        ));
        dl_url = remote.official.clone();
    }

    ui::info(format!("下载 {}", dl_url));
    if cache.exists() && cache.metadata()?.len() > 0 {
        ui::step("使用本地缓存");
    } else {
        let size = http::download(&dl_url, &cache, &ctx.cfg)?;
        ui::step(format!("已下载 {}", ui::human_size(size)));
    }

    ui::step(format!("解压到 {}", dest.display()));
    fsutil::extract_archive(&cache, &dest)?;

    if kind == Kind::Jdk && !fsutil::looks_like_jdk(&dest) {
        ui::warn("解压结果里没有 bin/java，该归档结构可能异常，请检查目录内容");
    }
    if kind == Kind::Maven && !dest.join("bin").exists() {
        ui::warn("解压结果里没有 bin 目录，请检查归档结构");
    }

    ui::ok(format!("{} {} 安装完成", kind.label(), remote.version));
    finish_install(ctx, kind, &remote.version, no_use)
}

fn finish_install(ctx: &Ctx, kind: Kind, version: &str, no_use: bool) -> Result<()> {
    let need = !no_use && (ctx.cfg.auto_use || tool::current(&ctx.paths, kind).is_none());
    if need {
        tool::use_version(&ctx.paths, kind, version)?;
        ui::ok(format!(
            "已切换到 {} {}（{} = {}）",
            kind.label(),
            version,
            kind.home_var(),
            ctx.paths.current_link(kind.as_str()).display()
        ));
    }
    Ok(())
}

fn parse_kind(s: &str) -> Result<Kind> {
    Kind::parse(s).ok_or_else(|| {
        anyhow::anyhow!("未知的工具类型: {}（可选 jdk / maven / gradle）", s)
    })
}
