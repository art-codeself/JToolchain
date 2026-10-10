//! 命令行定义。

use clap::{Args, Parser, Subcommand};

pub const BANNER: &str = "jtc - JToolchain CLI：统一管理 JDK / Maven / Gradle 版本";

#[derive(Parser, Debug)]
#[command(
    name = "jtc",
    version,
    about = BANNER,
    long_about = "jtc 采用「固定 PATH + current 链接」机制：\n\
                  环境变量里放的是 ...\\.jtc\\current\\jdk 这样的固定路径，\n\
                  切换版本只改变 current 的指向，因此 CMD / PowerShell / GitBash 全局一致，\n\
                  且终端无需重启。",
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Args, Debug)]
pub struct ToolCmd {
    #[command(subcommand)]
    pub action: ToolAction,
}

#[derive(Subcommand, Debug)]
pub enum ToolAction {
    /// 列出已安装版本
    #[command(name = "ls", alias = "list")]
    Ls {
        /// 列出远端可安装版本
        #[arg(long, short = 'r')]
        remote: bool,
    },

    /// 列出远端可安装版本
    #[command(name = "ls-remote", aliases = ["available", "lsr"])]
    LsRemote,

    /// 安装指定版本（如 21 / 21.0.5 / 3.9.6 / 8.5）
    #[command(name = "install", aliases = ["i", "add"])]
    Install {
        version: String,
        /// 安装后不自动切换
        #[arg(long)]
        no_use: bool,
    },

    /// 切换到指定版本
    #[command(name = "use", aliases = ["switch", "as"])]
    Use {
        /// 版本号；省略时读取项目 .jtoolchain
        version: Option<String>,
    },

    /// 卸载指定版本
    #[command(name = "uninstall", aliases = ["rm", "remove", "un"])]
    Uninstall {
        version: String,
        /// 跳过确认
        #[arg(long, short = 'y')]
        yes: bool,
    },

    /// 把本机已存在的安装目录登记进 jtc（不复制、不移动，只建链接）
    #[command(name = "link", alias = "register")]
    Link {
        /// 登记后的版本名，如 1.8.0_131 / 3.9.6 / 8.5
        version: String,
        /// 已存在的安装根目录
        path: String,
    },

    /// 扫描常见安装位置，发现本机已有的工具链
    #[command(name = "scan")]
    Scan {
        /// 扫描到的目录直接登记进 jtc
        #[arg(long)]
        link: bool,
    },

    /// 显示当前生效版本
    #[command(name = "current", alias = "cur")]
    Current,

    /// 打印当前版本的安装根目录（等价于 $JAVA_HOME 等）
    #[command(name = "home")]
    Home,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// 初始化 jtc：创建目录结构并把 jtc / java / mvn / gradle 写入用户 PATH
    Setup {
        /// 已初始化时强制重做
        #[arg(long)]
        force: bool,
        /// 改为写入系统 PATH 并**置于最前**，压过 JDK/Oracle 自带的 java 条目。
        /// 修改系统 PATH 需要管理员权限。
        #[arg(long, alias = "system-path")]
        system: bool,
    },

    /// 列出已安装版本（不带参数则列出全部三类）
    #[command(name = "ls", alias = "list")]
    Ls {
        kind: Option<String>,
        /// 列出远端可安装版本
        #[arg(long, short = 'r')]
        remote: bool,
    },

    /// 切换版本；省略参数时读取项目根 .jtoolchain 一次性切换全部
    Use {
        kind: Option<String>,
        version: Option<String>,
    },

    /// 显示当前生效版本
    Current { kind: Option<String> },

    /// 定位某个命令实际使用的可执行文件
    Which { exe: Option<String> },

    /// 打印需要设置的环境变量（供 shell 引用）
    Env {
        #[arg(long, default_value = "auto")]
        shell: String,
    },

    /// 在当前目录生成 .jtoolchain 项目配置
    Init {
        /// JDK 版本
        #[arg(long)]
        jdk: Option<String>,
        /// Maven 版本
        #[arg(long)]
        maven: Option<String>,
        /// Gradle 版本
        #[arg(long)]
        gradle: Option<String>,
    },

    /// 检查环境配置是否正确
    Doctor,

    /// 下载镜像管理
    Mirror {
        #[command(subcommand)]
        action: Option<MirrorAction>,
    },

    /// JDK 相关命令
    #[command(name = "jdk", aliases = ["java"])]
    Jdk(ToolCmd),

    /// Maven 相关命令
    #[command(name = "maven", alias = "mvn")]
    Maven(ToolCmd),

    /// Gradle 相关命令
    Gradle(ToolCmd),
}

#[derive(Subcommand, Debug)]
pub enum MirrorAction {
    /// 显示当前镜像
    Show,
    /// 列出所有镜像
    List,
    /// 切换镜像
    Set { name: String },
}
