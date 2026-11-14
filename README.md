# MIEdit — Ming-Install-Edit

**纯 Rust 实现的 Windows 安装包制作工具**——类 NSIS / Inno Setup 的现代替代品。

从一份 TOML 脚本（或 10 步 GUI 向导）+ 一组文件，编译出**自解压安装包 EXE**：现代化无边框
安装向导（支持**全窗口图片背景**）、**组件选择**（NSIS Section / Inno Components）、
**安装后任务**（Inno [Run]）、**NSIS 式自包含卸载器**（清单内嵌 EXE，自删除含目录清理）、
磁盘空间检测、运行中检测、UAC 提权、静默模式、键盘操作（Enter/Esc）等。

<p align="center">
  <img src="docs/screenshots/wizard.png" width="49%" alt="MIEdit 构建向导" />
  <img src="docs/screenshots/installer.png" width="49%" alt="MIEdit 安装向导" />
</p>
<p align="center">
  <img src="docs/screenshots/uninstaller.png" width="49%" alt="MIEdit 卸载器" />
</p>

> 设计经验取自 NSIS 与 Inno Setup 的**真实源码**（clone 至本地研究），
> 取长补短的分析见 [docs/lessons-from-nsis-inno.md](docs/lessons-from-nsis-inno.md)。

## 特性一览

| 能力 | 说明 |
| --- | --- |
| **GUI 构建向导** | HM NIS Edit / Inno Setup 式 **10 步向导**：欢迎 → 应用信息 → 安装程序选项（**实时预览**）→ 目录与授权 → 文件与组件 → 快捷方式 → 完成后运行 → 卸载与注册表 → **项目脚本**（查看/编辑/应用 `.miedit.toml`）→ 构建（wpReady 式摘要 + 一键构建） |
| 脚本驱动 | 一份 `.miedit.toml` 描述应用、文件、组件、UI 与快捷方式，与向导完全互通 |
| 自解压安装器 | 单文件 EXE：`[stub][payload][config][卸载器][背景图][图标][Trailer]` |
| 现代化 UI | egui 无边框窗口、玻璃卡片、主题色可配、自绘进度条、中文字体自适应 |
| **全窗口图片背景** | `background = "bg.png"`，`cover / contain / stretch` 三种模式 |
| 安装向导 | 欢迎 → 许可（可强制滚动到底）→ 组件选择 → 目录（**磁盘空间检测**）→ 安装进度（计数/速度/**细节开关**）→ 完成（**多任务复选**） |
| **组件选择** | NSIS Section / Inno Components 风格：`[[components]]` + 规则引用，必装锁定、全选/重置 |
| **安装后任务** | Inno [Run] 风格：`[[install.run_tasks]]` 完成页复选，支持 exe 参数与文档打开 |
| **NSIS 式卸载器** | **自包含**：清单与图标内嵌 `uninstall.exe`（MRISUNIN 尾部，无外部 json）；确认页（可勾选删除用户数据）→ 进度（细节默认收起可展开）→ 完成 |
| **自删除** | 自身移入 %TEMP% + 分离 cmd：等待退出 → 删除自身 → `rd /s /q` 清理安装目录 → 重试 |
| 卸载注册表 | HKLM/HKCU `...\Uninstall\{app_id}`（控制面板可见），支持自定义注册表键值 |
| 快捷方式 | 开始菜单（自动含“卸载”快捷方式）与桌面，per-machine / per-user 自适应 |
| **默认图标** | 未配置图标时自动使用内置 **modern-install.ico / modern-uninstall.ico**（NSIS 做法），PE 资源 + 窗口图标同步替换 |
| 磁盘空间检测 | 目录页显示需要/可用空间，不足禁用安装 |
| 运行中检测 | 安装前独占探测主程序占用，重试/继续 |
| 升级检测 | 发现旧版本记录版本号并静默卸载后升级（`InstallDirRegKey` 式目录记忆） |
| 权限 | `per_machine = true` 启动时自动 UAC 提权重启 |
| 静默模式 | 安装器/卸载器均支持 `--silent`（NSIS `/S` 风格）与 `--dir=`，可自动化测试 |
| 压缩 | `deflate`（条目级）或 `solid`（整包 Deflate，NSIS solid 风格，体积更小） |
| Unicode | 中文路径 / 文件名 / UI 全支持 |
| 纯 Rust | 无 C 构建链依赖（egui / flate2 / walkdir / winreg / windows-rs） |

## 快速开始

```bash
cargo build --release
# target/release/: miedit-builder.exe miedit-compiler.exe miedit-installer.exe miedit-uninstaller.exe
```

### 用 GUI 构建向导制作（推荐）

```bash
target/release/miedit-builder.exe
# 或直接打开示例项目（跳到第 2 步）
target/release/miedit-builder.exe examples/demo-app.miedit.toml
```

### 用 CLI 编译

```bash
target/release/miedit-compiler.exe build examples/demo-app.miedit.toml -o out/MIEdit-Demo-Setup.exe
```

### 安装 / 卸载

```bash
out/MIEdit-Demo-Setup.exe                        # GUI 安装
out/MIEdit-Demo-Setup.exe --silent               # 静默安装（日志 %TEMP%\miedit-install.log）
out/MIEdit-Demo-Setup.exe --silent --dir="C:\My Apps\Demo"

"%LOCALAPPDATA%\Programs\MIEdit Demo\uninstall.exe"             # GUI 卸载（可勾选删除用户数据）
"%LOCALAPPDATA%\Programs\MIEdit Demo\uninstall.exe" --silent    # 静默卸载（保留用户数据）
```

## 脚本参考（`.miedit.toml`）

完整示例见 [`examples/demo-app.miedit.toml`](examples/demo-app.miedit.toml)。

```toml
[app]
name = "MIEdit Demo"          # 应用名（必填）
version = "1.2.0"
publisher = "MIEdit Project"
url = "https://example.com"   # 可选，写入注册表 URLInfoAbout
exe = "MyApp.exe"             # 主程序（包内相对路径）
icon = "app.ico"              # 可选；缺省使用内置 modern-install.ico
per_machine = false           # true = 每机器（HKLM + 公共开始菜单 + UAC）

[install]
dir_template = "{autopf}\\{app_name}"      # 占位符：{autopf} {localappdata} {appdata} {desktop} {docs} {app_name}
compression = "deflate"                    # deflate（条目级）| solid（整包 Deflate，体积更小）
user_data_dirs = ["{appdata}\\{app_name}"] # 卸载时可勾选删除
uninstall_icon = "uninstall.ico"           # 可选；缺省使用内置 modern-uninstall.ico

[[install.run_tasks]]         # Inno [Run] 风格：完成页复选框
name = "运行 {app}"
exe = "MyApp.exe"

[ui]
window_size = [960.0, 640.0]
background = "bg.png"         # ★ 全窗口图片背景
background_style = "cover"
accent = "#3B82F6"
license = "LICENSE.txt"
license_scroll_required = true
show_components_page = true
[ui.text]                     # UI 文案覆盖（键见下表），支持 {app} {version} {file} {size} {free}

[shortcuts]
start_menu = true             # 自动含“卸载”快捷方式
desktop = true

[[files]]                     # 文件 / 目录 / 通配符（支持 ** 递归）
source = "payload/*"
dest = ""                     # 包内目标子目录（空 = 安装根目录）

[[components]]                # NSIS Section / Inno Components
name = "核心程序"
description = "主程序（必需）"
required = true               # 必装：勾选锁定
files = [0, 2]                # 引用 [[files]] 规则索引；未被引用的规则始终安装

[[registry]]
key = "HKCU\\Software\\Vendor\\App"   # HKCU / HKLM 前缀
values = { InstallPath = "{install_dir}" }
```

## 架构

```
crates/
├── miedit-core         # 包格式（MIEARC2 归档 + MIEDPKG2 包尾 + MIEDUNIN 卸载内嵌）、配置、Windows 工具
├── miedit-compiler     # 打包库（build_package）+ CLI（含 PE 图标替换 UpdateResourceW）
├── miedit-builder      # GUI 构建向导（10 步，实时预览，脚本编辑）
├── miedit-installer    # 自解压安装器 Stub（egui 向导）
└── miedit-uninstaller  # 自包含卸载器 Stub（egui）
```

成品安装包布局：

```text
[stub 安装器 EXE][payload 归档][config JSON][卸载器 EXE][背景图][图标][Trailer 104B]
卸载器 EXE 尾部：[卸载清单 JSON][卸载图标][MIEDUNIN footer 24B]
```

## 测试

```bash
cargo test --workspace    # 10 个单元测试：归档/包尾/通配符/路径安全/icon 解析/组件过滤映射/slug
```

已在 Windows 10 x64 实机验证：CLI 与 GUI 构建链路、静默安装/卸载全清理、组件过滤生效、
默认图标链路（未配置自动内嵌）、solid 压缩（592KB→103KB）、升级检测、旧格式兼容。

## License

MIT
