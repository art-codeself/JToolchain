# JToolchain CLI (`jtc`)

统一管理 **JDK / Maven / Gradle** 版本的命令行工具，用法对标 `nvm`，一个命令管三套工具链。

```bash
jtc jdk install 21        # 安装 JDK 21
jtc jdk use 21            # 切换到 JDK 21
jtc maven install 3.9.6
jtc gradle use 8.5
jtc use                   # 按项目 .jtoolchain 一键对齐
```

---

## 一、为什么不用 nvm 那套 shell function

`nvm` 是 shell 函数，靠修改**当前 shell 的 PATH** 工作，所以必须往 `.bashrc` / `.zshrc` 注入代码，且 Windows CMD 基本无法实现。

`jtc` 采用 **固定 PATH + current 链接**（nvm-windows 的思路，但覆盖三类工具）：

```
%JTC_HOME%\current\jdk     ->  %JTC_HOME%\jdk\21.0.5+11     (目录链接)
%JTC_HOME%\current\maven   ->  %JTC_HOME%\maven\3.9.6
%JTC_HOME%\current\gradle  ->  %JTC_HOME%\gradle\8.5
```

环境变量**只写一次，永不改动**：

| 变量 | 固定值 |
| --- | --- |
| `JAVA_HOME` | `<JTC_HOME>\current\jdk` |
| `MAVEN_HOME` / `M2_HOME` | `<JTC_HOME>\current\maven` |
| `GRADLE_HOME` | `<JTC_HOME>\current\gradle` |
| `PATH` | 追加 `\current\jdk\bin`、`\current\maven\bin`、`\current\gradle\bin`、`\bin` |

> 这些值在 `setup` 时写成**展开后的绝对路径**而非 `%JTC_HOME%\...`。
> 并非所有宿主都会展开 PATH / JAVA_HOME 里的变量（部分 IDE 内置终端、老式启动器就不展开），
> 写死路径能保证 `jtc` / `java` 在任何终端里都能被找到。

切换版本 = 重建 `current` 链接。好处：

- **跨 shell 一致**：CMD、PowerShell、GitBash、WSL、IDE 终端行为完全相同
- **终端不用重启**：已打开的窗口里 `JAVA_HOME` 展开后就是 `...\current\jdk`，链接一改立即生效
- **无 shell 注入**：不用往 `.bashrc` 里塞东西，不拖慢终端启动
- Windows 用 **junction**（`mklink /J`），不需要管理员权限，也不会遇到符号链接权限问题

---

## 二、安装

### 从源码构建

```bash
cargo build --release     # 产物 target/release/jtc.exe
jtc setup                 # 初始化：建目录、写环境变量、把 jtc 复制进 PATH
```

`setup` 会：

1. 创建 `%USERPROFILE%\.jtc`（可用 `JTC_HOME` 环境变量改位置）
2. 把 `jtc.exe` 复制到 `%JTC_HOME%\bin`
3. 写入 `JTC_HOME` / `JAVA_HOME` / `MAVEN_HOME` / `M2_HOME` / `GRADLE_HOME`
4. 把 4 个目录追加进用户 `PATH`
5. 广播 `WM_SETTINGCHANGE`，让新开的终端立即拿到新环境

> 重新打开一个终端执行 `jtc doctor` 验证。

---

## 三、命令

### 通用

| 命令 | 说明 |
| --- | --- |
| `jtc setup [--force]` | 初始化 / 修复环境 |
| `jtc setup --system` | 把 jtc 写到**系统 PATH 最前**（需管理员），用于压过 JDK 安装器自带的 java 条目 |
| `jtc ls [jdk\|maven\|gradle]` | 列出已安装版本（省略参数列全部） |
| `jtc ls jdk -r` | 列出远端可安装版本 |
| `jtc use` | 按项目 `.jtoolchain` 一次切换全部 |
| `jtc use jdk 21` | 切换单个工具 |
| `jtc current [kind]` | 显示当前生效版本（附带 `java -version` 实测） |
| `jtc which [java\|mvn\|gradle]` | 定位实际可执行文件 |
| `jtc env [--shell powershell]` | 输出环境变量片段，供 `Invoke-Expression` / `eval` |
| `jtc init [--jdk 21] [--maven 3.9.6]` | 生成项目级 `.jtoolchain` |
| `jtc doctor` | 环境自检 |
| `jtc mirror [show\|list\|set <name>]` | 下载镜像管理 |

### 每类工具（`jdk` / `maven` / `gradle`）

| 命令 | 别名 | 说明 |
| --- | --- | --- |
| `jtc jdk ls` | `list` | 已安装版本 |
| `jtc jdk ls-remote` | `available` | 远端版本 |
| `jtc jdk install 21` | `i`、`add` | 安装（可写 `21` / `21.0.5` / `21.0.5+11`） |
| `jtc jdk use 21` | `switch` | 切换 |
| `jtc jdk link <版本> <路径>` | `register` | 登记本机已有的安装目录（不复制、不移动） |
| `jtc jdk scan [--link]` | | 扫描常见位置，发现本机已装的工具 |
| `jtc jdk uninstall 21` | `rm`、`remove` | 卸载（若正被使用，自动回退到剩余最高版本） |
| `jtc jdk current` | `cur` | 当前版本 |
| `jtc jdk home` | | 打印根目录（等价 `$JAVA_HOME`） |

`jdk` 可写作 `java`，`maven` 可写作 `mvn`。

---

## 四、项目级锁定

在项目根目录生成 `.jtoolchain`：

```bash
jtc init --jdk 21 --maven 3.9.6 --gradle 8.5
```

```ini
# JToolchain 项目配置
jdk=21
maven=3.9.6
gradle=8.5
```

提交到仓库后，团队成员 clone 下来只需一条命令：

```bash
jtc use
```

`jtc` 会向上查找 `.jtoolchain`（遇到 `.git` 停止），一次性把三套工具链对齐。

---

## 五、接管本机已安装的工具

已经装好的 JDK / Maven / Gradle 不必重装，直接登记进 jtc —— **只建链接，不复制、不移动、不占额外空间**。

```bash
jtc jdk scan                                              # 先看看本机有什么
jtc jdk scan --link                                       # 一次性全部登记
jtc jdk link 1.8.0_131 "C:\Program Files\Java\jdk1.8.0_131"   # 或手动登记单个
jtc jdk use 1.8                                           # 登记后即可自由切换
```

`scan` 覆盖的常见位置：`Program Files\Java`、Eclipse Adoptium、Amazon Corretto、Zulu、
Microsoft、BellSoft、GraalVM、Semeru、Scoop、Chocolatey、sdkman。

- JDK 的识别标准是**同时存在 `bin/java` 和 `bin/javac`**，因此纯 JRE 不会被误登记
- 版本号从目录名推断（`jdk1.8.0_131` → `1.8.0_131`，`apache-maven-3.9.6` → `3.9.6`）
- 卸载登记进来的版本时，**只删除链接，原目录及其文件完好无损**

---

## 六、镜像

默认 `tsinghua`，国内下载 Adoptium / Maven 通常能跑满带宽。

```bash
jtc mirror list
jtc mirror set tencent
jtc mirror set default      # 官方源
```

| 镜像 | JDK | Maven | Gradle |
| --- | --- | :-: | :-: |
| `default` | 官方 | 官方 | 官方 |
| `tsinghua` | ✔ | ✔ | ✔ |
| `tencent` | 走清华 | ✔ | ✔ |
| `aliyun` | 官方 | ✔ | 官方 |
| `huawei` | 官方 | ✔ | 官方 |

配置保存在 `%JTC_HOME%\settings.json`，可直接编辑：

```json
{
  "mirror": "tsinghua",
  "jdk_vendor": "eclipse",
  "jdk_impl": "hotspot",
  "auto_use": true,
  "no_progress": false,
  "timeout_secs": 30
}
```

`jdk_vendor` 可选 `eclipse`(Temurin) / `ibm`(Semeru) / `zulu` / `microsoft` / `oracle`；`jdk_impl` 可选 `hotspot` / `openj9`。

---

## 七、目录结构

```
%JTC_HOME%\
├── jdk\21.0.5+11\
├── jdk\17.0.13+11\
├── maven\3.9.6\
├── gradle\8.5\
├── current\
│   ├── jdk     -> ..\jdk\21.0.5+11
│   ├── maven   -> ..\maven\3.9.6
│   └── gradle  -> ..\gradle\8.5
├── bin\jtc.exe
├── cache\            # 下载缓存，可安全删除
└── settings.json
```

---

## 八、常见问题

**Q：`jtc use` 已经切了，`java -version` 却还是旧版本？**
PATH 里有别的 java 目录排在 jtc 前面。安装 JDK 时安装器会把
`...\jdk1.8.0_131\bin`、`C:\ProgramData\Oracle\Java\javapath` 一类条目写进**系统 PATH**，
而 Windows 的拼接顺序是「系统 PATH + 用户 PATH」，jtc 的条目在用户 PATH 里，必然被盖住。

```bash
jtc doctor                 # 会直接告诉你被哪个目录抢走了优先命中
jtc setup --system         # 管理员终端执行，把 jtc 提到系统 PATH 最前
```

> `doctor` 会逐条列出 `java` / `mvn` / `gradle` 的真实命中位置，
> 显示 `[!!] xxx 实际用的还是 <路径>` 就说明被抢占了。

**Q：切换后 IDE 还是旧版本？**
IntelliJ / Eclipse 在启动时会解析 JDK 实际路径并缓存。切换后重启 IDE，或在 IDE 里重新选择 `...\.jtc\current\jdk`。

**Q：`jtc` 自身不在 PATH？**
执行 `jtc setup` 重新写入；若 `bin` 目录里的 `jtc.exe` 正在运行导致复制失败，手动复制一次即可。

**Q：卸载了正在使用的版本会怎样？**
`jtc` 会自动把 current 回退到剩余的最高版本，不会出现 `JAVA_HOME` 悬空。

**Q：命令提示「不是内部或外部命令」？**
先确认是新开的终端（旧窗口的环境块是 `setup` 之前的快照，必须关掉重开）。
若重开仍不行，执行 `jtc setup --force` 重写环境变量——它会清理旧条目再写入，不会重复累积。

**Q：换了 JTC_HOME 的位置怎么办？**
环境变量里写的是绝对路径，迁移目录后重跑一次 `jtc setup` 即可同步（旧条目自动清理）。

**Q：需要管理员权限吗？**
不需要。目录链接用 junction，环境变量写 `HKCU\Environment`（仅当前用户）。

---

## 九、与 nvm / sdkman 对照

| | nvm | sdkman | **jtc** |
| --- | :-: | :-: | :-: |
| 管理对象 | Node | JVM 生态 | JDK + Maven + Gradle |
| 切换机制 | shell function | shell function | current 目录链接 |
| 需要 shell 注入 | 是 | 是 | **否** |
| Windows 支持 | 需 nvm-windows | 弱 | **原生优先** |
| 终端需重启 | 否（同 shell） | 否 | **否（全 shell）** |
| 项目级锁版本 | `.nvmrc` | `.sdkmanrc` | `.jtoolchain` |
| 运行时依赖 | Node | JVM | **无（单文件 exe）** |
