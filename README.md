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
| **GUI 构建向导** | HM NIS Edit / Inno Setup 式 **10 步向导 + 左栏步骤树**（全部步骤可点直达、当前高亮、完成 √）：欢迎 → 应用信息 → 安装程序选项（**实时预览**）→ 目录与授权 → 文件与组件 → 快捷方式 → 完成后运行 → 卸载与注册表 → **MI 脚本 IDE**（深度对齐 VSCode，见下）→ 构建（wpReady 式摘要 + 一键构建） |
| **MI 脚本 IDE** | VSCode 式编辑器：文件标签（未保存圆点）+ 行号（**当前行行号亮色**）+ Dark+ 语法高亮 + **查找/替换**（Ctrl+F/H，命中计数 n/N、逐个跳转、大小写开关）+ **命令面板**（Ctrl+Shift+P）+ **转到行**（Ctrl+G）+ **minimap**（点击跳行）+ **面包屑**（`文件 › [当前节]`）+ **状态栏**（Ln/Col、选中、缩进风格、UTF-8、错误计数）+ **自动缩进/Tab 转空格/括号引号补全** + Alt+Z 自动换行 + 错误行红标 + 实时校验问题面板 |
| 脚本驱动 | 一份 `.mis`（MI 脚本，自研轻量脚本语言，TOML 语法）描述应用、文件、组件、UI 与快捷方式，与向导完全互通；**v2 组件指令**（NSIS Section 指令语义）：组件表内 `code = [{ op = "run" / "write_reg" / "create_dir" / "delete_file" / "if_file_exists" / "abort" / … }]`（**13 条指令**，含注册表全类型/ExecShell/RMDir/条件分支），勾选组件按序执行（未选不执行），路径支持 `$INSTDIR`/`$DESKTOP`/`$SMPROGRAMS`/`$APPDATA`/`$TEMP`/`$WINDIR` 等 12 个 NSIS 常量变量，副作用自动登记卸载清单精确清理 |
| 自解压安装器 | 单文件 EXE：`[stub][payload][config][卸载器][背景图][图标][Trailer]` |
| 现代化 UI | egui 无边框窗口、玻璃卡片、主题色可配、自绘进度条、中文字体自适应 |
| **全窗口图片背景** | `background = "bg.png"`，`cover / contain / stretch` 三种模式 |
| 安装向导 | 欢迎 → 许可（可强制滚动到底）→ 组件选择（**Space required 联动**）→ 目录（**磁盘空间检测**）→ 安装进度（计数/速度/**细节开关**）→ 完成（**多任务复选**） |
| **组件选择** | NSIS Section / Inno Components 风格：`[[components]]` + 规则引用，必装锁定、全选/重置；**分组树**：`group = "父组/子组"`（NSIS `SectionGroup`），组件页树形渲染、组行折叠/展开、组复选框三态联动 |
| **安装类型** | NSIS `InstallTypes` / Inno `[Types]` 对齐：`[[install_types]]` 组件页顶部单选（Typical/Full/…），切换类型联动组件勾选；手动改动任意组件自动切到 **Custom** |
| **组件描述侧栏** | NSIS 组件页描述框对齐：悬停/点击组件行，下方描述面板跟随显示 `description`，必装组件带「必需」徽标，空描述显示占位 |
| **安装后任务** | Inno [Run] 风格：`[[install.run_tasks]]` 完成页复选，支持 exe 参数与文档打开 |
| **NSIS 式卸载器** | **自包含**：清单与图标内嵌 `uninstall.exe`（MRISUNIN 尾部，无外部 json）；确认页（可勾选删除用户数据）→ 进度（细节默认收起可展开）→ 完成 |
| **自删除** | 自身移入 %TEMP% + 分离 cmd：等待退出 → 删除自身 → `rd /s /q` 清理安装目录 → 重试 |
| **自定义命令** | NSIS `Exec`/`ExecWait` 对齐：`[[install.exec_before]]`（解包前）/ `[[install.exec_after]]`（清单就绪后）/ `[[install.exec_before_uninstall]]`（卸载前，如停服务）；`abort_on_error`/`optional` 开关，`{install_dir}` 模板，GUI 与静默同生效，Win 下不闪黑框 |
| 卸载注册表 | HKLM/HKCU `...\Uninstall\{app_id}`（控制面板可见）；自定义注册表支持**全类型**（NSIS `WriteRegStr/DWORD/Bin/ExpandStr/MultiSz` 对齐）与 32/64 位视图（`SetRegView`）；卸载**精确清理**：新建键整树删、既有键只删本次写入的值 |
| 快捷方式 | 开始菜单（自动含“卸载”快捷方式）与桌面，per-machine / per-user 自适应 |
| **默认图标** | 未配置图标时自动使用内置 **modern-install.ico / modern-uninstall.ico**（NSIS 做法），PE 资源 + 窗口图标同步替换 |
| **EXE 版本信息** | 安装器/卸载器 EXE 嵌入 VERSIONINFO（NSIS `VIProductVersion` / `VIAddVersionKey` 对齐）：产品名/版本/公司/描述/版权，资源管理器属性可见 |
| 磁盘空间检测与目录校验 | 目录页可用空间**随所选驱动器实时刷新**（NSIS DirectoryPage），无效路径/不存在盘符拒绝前进；`allow_root_dir_install` 根目录安装开关（NSIS `AllowRootDirInstall` 默认禁根目录） |
| **步骤导航** | HM NIS Edit 向导式**左列步骤导航栏**：当前步高亮/已完成勾点，`ui.show_step_rail` 可关 |
| **界面模板** | 五套内置模板一键切换：`ui.template = aurora / midnight / sunrise / forest / mono` |
| **液态玻璃** | `ui.material = "glass"` 半透明卡片 + 高光描边（默认）；`ui.acrylic` 启用 Win11 亚克力背景板（失败自动回退） |
| **Linux 适配** | 安装器/卸载器 Linux 可用：XDG 路径模板、`.desktop` 快捷方式、`uninstall.json` 清单卸载、CJK 字体回退；CI 产出 Linux zip |
| **命令行安装** | `--silent` `--dir=` `--components=` `--no-shortcuts` `--lang=` `--repair` `--help` `--version`，退出码 0/1/2（NSIS `/S` + Inno `/COMPONENTS` 语义） |
| **单实例互斥** | NSIS CreateMutex 惯例 / Inno `AppMutex` 对齐：安装器/卸载器 `Global\MIEDIT-<app_id>[-uninstall]` 命名互斥，已有实例运行时 GUI 弹框/静默日志 + 退出（多会话覆盖） |
| **语言命令行覆盖** | NSIS `/LANG=<十进制 LCID>` 语义：`--lang=2052` / `/LANG=1033` / `0x` 十六进制 / 语言 id 均可，GUI+静默同生效，优先于自动预选与清单记录语言，无效值宽容回退 |
| **UAC 执行级别清单** | NSIS `RequestExecutionLevel` 对齐：`install.request_execution_level = user/highest/admin`（缺省按 per_machine 推导），编译器向安装器/卸载器 EXE 嵌入 RT_MANIFEST（requestedExecutionLevel + supportedOS Vista…Win11 + DPI/长路径感知），非法级别构建期报错 |
| 运行中检测 | 安装前独占探测主程序占用，重试/继续 |
| 升级检测 | 发现旧版本记录版本号并静默卸载后升级（`InstallDirRegKey` 式目录记忆） |
| 权限 | `per_machine = true` 启动时自动 UAC 提权重启 |
| 静默模式 | 安装器/卸载器均支持 `--silent`（NSIS `/S` 风格）与 `--dir=`，可自动化测试 |
| **全流程实时预览** | IDE「▶ 预览安装 / ▶ 预览卸载」按键：从欢迎至解锁安装（许可勾选才放行）到完成页的**交互式**全流程预览（目录可改/组件可勾/进度动画/日志滚动），等比缩放渲染随脚本实时变化（NSIS/HM NIS Edit 向导预览语义） |
| **语法检测和纠错** | VSCode Problems 式问题面板：未知指令（就近建议）/ 缺必填字段 / dword 非法 / hive 缺前缀 / Tab 缩进 / 引号不闭合 / TOML 错误定位，可行诊断带**一键修复**按钮（`miedit_core::midiag` 三层诊断，纯函数可单测） |
| **深色/浅色主题** | 默认**跟随系统**（Windows 注册表 / Linux gsettings 探测），设置菜单可切 深色/浅色/跟随系统（持久化）；编辑器同步 Dark+/Light+ 配色，预览页恒定模拟安装器深色视觉 |
| **打开速度** | 安装器 GUI 关键路径只解析包尾+配置+图标：CRC32 校验移后台（提取前 `CrcGate` 把关，损坏包仍拒绝安装、`--ncrc` 不变），壁纸解码后台化（先渐变后壁纸）；release 实测 256MB 包关键路径 637ms→0ms |
| **覆盖策略** | NSIS `SetOverwrite` 对齐：`on / off / try / ifnewer`；安装后保留源文件时间戳，`ifnewer` 仅在源更新时覆盖 |
| **快捷方式增强** | Inno `[Icons]` 对齐：`[[shortcuts.entries]]` 自定义名称/子目录/参数/图标/工作目录/描述，位置多选（开始菜单/桌面/快速启动栏），与简单开关叠加兼容 |
| **最小系统版本门槛** | Inno `MinVersion` 对齐：`install.min_version`，RtlGetVersion 取真实版本（不受兼容性垫片影响），低于门槛启动即拒 |
| **多语言向导** | NSIS MUI2 `MUI_LANGUAGE` 对齐：`[[ui.languages]]` 语言包（id + 显示名 + 文案覆盖），内置 zh-CN / en-US 双语包，>1 语言时欢迎页前显示语言选择页，按系统 UI 语言自动预选；`ui.text` 仍是最高优先级单键覆盖 |
| **卸载器多语言** | NSIS `MUI_UNPAGE_LANGUAGE` 对齐，步骤 28 起**语言跟随安装**：安装时选定的语言 id 随清单记录，卸载器直接使用（装的时候选 English → 卸载即英文，零提问）；确认/进度/完成/日志全部随语言切换（旧包无语言包 → 保持中文，行为不变；旧清单多语言回退语言选择页） |
| **环境变量** | NSIS WriteEnvStr 惯例 / Inno ChangesEnvironment 对齐：`[[install.env]]` 写 HKCU/HKLM 环境区，`path_append`/`path_prepend` 按 `;` 列表去重合并，写入后广播 `WM_SETTINGCHANGE`（无需注销）；卸载精确清理：PATH 只移除本安装写入的段，直接设值还原安装前原值 |
| **卸载清单 NSIS 语义** | UninstallLog/LIFO：文件与快捷方式按安装**相反顺序**删除；目录 **`RMDir` 语义——仅删除空目录**，非空保留并记录日志（不算失败，绝不碰清单外内容）；缺失项静默跳过（`Delete` 语义） |
| **卸载统计** | NSIS 卸载完成统计对齐：完成页展示已删除文件数/目录数/释放空间（含用户数据目录递归统计） |
| **覆盖失败重试交互** | NSIS 文件错误对话框对齐：安装中文件被占用写不进时弹出「重试 / 忽略 / 中止」模态卡片——重试循环、忽略记录日志并继续、中止说明已写入文件去向；静默模式自动忽略并计划重启替换 |
| **安装期重启替换** | NSIS `SetRebootFlag` / Inno `restartreplace` 对齐：被占用文件先改名腾位立即写入新内容，旧文件调度重启删除（`PendingFileRenameOperations`）；完成页琥珀提示“需要重启以完成安装”并列出文件 |
| **重启删除** | NSIS `Delete /REBOOTOK` 对齐：卸载时被占用的文件 `MoveFileEx` 计划重启删除，完成页/静默日志提示 |
| **卸载运行检测** | 卸载确认页检测主程序占用（可重新检测），静默模式记录警告后继续 |
| **QuietUninstallString** | 卸载注册表写入静默卸载入口（ARP 自动化 / NSIS 风格） |
| **修复模式** | Inno `ModifyPath` 对齐：卸载注册表写 `ModifyPath`，控制面板「更改/修复」按钮启动安装器 `--repair` 修复向导——跳过向导页直达确认页（含文件/快捷方式/注册表统计），确认后全量重装文件并重建快捷方式/注册表 |
| **完成详情停留页** | NSIS `MUI_PAGE_INSTFILES` 对齐：安装/卸载完成后停留在详情页（完成状态 + 全量日志），点「下一步」才进入完成页 |
| **卸载器壁纸** | `install.uninstall_background` 配置卸载向导背景图（编译期内嵌进卸载器 EXE），全窗铺满 + 半透明遮罩；缺省内置渐变 |
| **install.log** | NSIS `LogSet` 风格安装日志落盘到 `$INSTDIR\install.log`（`install.write_log` 可关） |
| **包完整性校验** | NSIS `CRCCheck` 对齐：安装包尾写入 payload/config/uninstaller 三分区 CRC32（MIEDPKG3），启动时流式校验，损坏包报"安装包已损坏"；`--ncrc` / `/ncrc` 跳过；旧格式包仍可安装 |
| 压缩 | 十四档（NSIS `SetCompressor` 对齐 + 扩展，全部纯 Rust）：`deflate`（条目级）· `solid`（整包 Deflate）· `bzip2` / `bzip2-solid`（NSIS 经典算法，libbz2-rs 纯 Rust）· `lzma` / `lzma-solid`（NSIS 旗舰 LZMA，tukaani xz-for-java 纯 Rust 移植）· `brotli` / `brotli-solid`（现代算法）· `zstd` / `zstd-solid`（Inno 扩展对齐，mzstd 纯 Rust）· **激进档** `lzma-ultra` / `lzma-ultra-solid`（preset 9 + 64MB 字典 + nice_len 273）与 `zstd-ultra` / `zstd-ultra-solid`（level 22），兼容别名 bz2/zst/zstandard/lzma-9/zstd-22；混合语料 2.86MB 实测：lzma 9.8%（体积最小）· zstd 15.5%（解压 61ms 最快）· bzip2 15.3% · deflate 21.8%；ultra 仅改编码参数、输出格式自描述，解码端零改动，小语料增益有限请按包体量选档 |
| Unicode | 中文路径 / 文件名 / UI 全支持 |
| 纯 Rust | 无 C 构建链依赖（egui / flate2 / brotli / lzma-rust / bzip2→libbz2-rs / mzstd / walkdir / winreg / windows-rs） |

## 快速开始

```bash
cargo build --release
# target/release/: miedit-builder.exe mis.exe miedit-installer.exe miedit-uninstaller.exe
```

### 用 GUI 构建向导制作（推荐）

```bash
target/release/miedit-builder.exe
# 或直接打开示例项目（跳到第 2 步）
target/release/miedit-builder.exe examples/demo-app.mis
```

### 用 CLI 编译

```bash
target/release/mis.exe build examples/demo-app.mis   # 输出目录默认 <当前目录>/output/（可 -o 指定或脚本 [build] output_dir）
```

### 安装 / 卸载

```bash
out/MIEdit-Demo-Setup.exe                        # GUI 安装
out/MIEdit-Demo-Setup.exe --silent               # 静默安装（日志 %TEMP%\miedit-install.log）
out/MIEdit-Demo-Setup.exe --silent --ncrc        # 静默安装并跳过包 CRC32 校验（NSIS /NCRC 语义）
out/MIEdit-Demo-Setup.exe --silent --dir="C:\My Apps\Demo"

"%LOCALAPPDATA%\Programs\MIEdit Demo\uninstall.exe"             # GUI 卸载（可勾选删除用户数据）
"%LOCALAPPDATA%\Programs\MIEdit Demo\uninstall.exe" --silent    # 静默卸载（保留用户数据）
```

## 脚本参考（`.mis` — MI 脚本语言）

完整示例见 [`examples/demo-app.mis`](examples/demo-app.mis)。

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
dir_template = "{autopf}\\{app_name}"      # 占位符：{autopf} {autopf86} {localappdata} {appdata} {desktop} {docs} {app_name}
compression = "deflate"                    # deflate|solid|lzma|lzma-solid|brotli|brotli-solid（NSIS SetCompressor 对齐 + 扩展）
overwrite = "on"                           # NSIS SetOverwrite：on（默认）| off | try | ifnewer
write_log = true                           # 安装日志写入 $INSTDIR\\install.log（NSIS LogSet 风格）
user_data_dirs = ["{appdata}\\{app_name}"] # 卸载时可勾选删除
uninstall_icon = "uninstall.ico"           # 可选；缺省使用内置 modern-uninstall.ico
min_version = "6.1"                        # 可选；最低系统版本门槛（如 6.1 = Win7，10.0 = Win10）

[[install.env]]               # NSIS WriteEnvStr 惯例 / Inno ChangesEnvironment（可选）
name = "MIEDIT_HOME"          # 变量名
value = "{install_dir}"       # 值（{install_dir} 可用）
part = "all"                  # all 直接设值 | path_append 追加 | path_prepend 前置（";" 列表去重）
scope = "user"                # user（HKCU，默认）| machine（HKLM，需管理员）

[[install.env]]
name = "Path"
value = "{install_dir}"
part = "path_append"          # 卸载时只移除本安装写入的段，不动其他软件的段
scope = "user"

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
[ui.text]                     # UI 文案覆盖（键见下表），支持 {app} {version} {file} {size} {free}；优先级最高
language_auto_detect = true   # 按系统 UI 语言自动预选（默认 true）

[[ui.languages]]              # NSIS MUI2 MUI_LANGUAGE 风格语言包（可选，>1 个时显示语言选择页）
id = "zh-CN"                  # BCP 47 风格 id；与内置包匹配时缺键自动补全
name = "简体中文"             # 语言选择页显示名
[ui.languages.text]           # 该语言的文案覆盖（未覆盖键由内置 zh-CN 包补全）
next = "下一步"

[[ui.languages]]
id = "en-US"
name = "English"

[shortcuts]
start_menu = true             # 自动含“卸载”快捷方式
desktop = true

[[shortcuts.entries]]         # Inno [Icons] 风格快捷方式条目（可选，与上方开关叠加）
name = "使用指南"             # 快捷方式名称
exe = "docs/guide.md"        # 目标（包内相对路径，空 = 主程序）
subfolder = "工具"            # 开始菜单子目录（空 = 应用名分组目录）
icon = "docs/icon.ico"       # 图标（包内相对路径，空 = 目标自身）
working_dir = ""              # 工作目录（空 = 目标所在目录，支持 {install_dir}）
comment = "查看使用指南"      # 悬停提示
locations = ["desktop"]       # 位置多选：start_menu / desktop / quick_launch

[[files]]                     # 文件 / 目录 / 通配符（支持 ** 递归）
source = "payload/*"
dest = ""                     # 包内目标子目录（空 = 安装根目录）

[[components]]                # NSIS Section / Inno Components
name = "核心程序"
description = "主程序（必需）"
required = true               # 必装：勾选锁定
files = [0, 2]                # 引用 [[files]] 规则索引；未被引用的规则始终安装
code = [                      # MI 脚本 v2 组件指令（NSIS Section 指令语义，可选）：
                              # 组件勾选后按序执行，未选不执行；失败记警告继续
                              # （run.abort_on_error = true 与 abort 指令显式中止）
                              # 指令集 13 条（NSIS 对齐）：run（Exec/ExecWait）/
                              # write_reg / create_dir / delete_file / rename（Rename）/
                              # copy_files（CopyFiles，目录树递归）/
                              # write_ini（WriteINIStr，节键不区分大小写）/
                              # sleep（Sleep 毫秒） / detail_print /
                              # exec_shell（ExecShell，动词打开 URL/文档）/
                              # rmdir（RMDir / RMDir /r）/
                              # if_file_exists（IfFileExists，then/else 嵌套，深度 ≤ 4）/
                              # abort（Abort 中止安装）
  { op = "detail_print", text = "正在注册核心组件…" },
  { op = "create_dir", path = "$INSTDIR\\logs" },       # NSIS CreateDirectory
  { op = "delete_file", path = "$INSTDIR\\old.cfg" },   # NSIS Delete（缺失静默跳过）
  { op = "write_ini", file = "$INSTDIR\\cfg.ini",       # NSIS WriteINIStr
    section = "Main", key = "Mode", value = "fast" },
  { op = "copy_files", from = "$INSTDIR\\docs",         # NSIS CopyFiles
    to = "$APPDATA\\Demo\\docs" },
  { op = "write_reg", key = "HKCU\\Software\\Demo",     # NSIS WriteRegStr/WriteRegDWORD/WriteRegBin/
    name = "Path", value = "$INSTDIR", type = "str" },  # WriteRegExpandStr/WriteRegMultiSz：
  { op = "write_reg", key = "HKCU\\Software\\Demo",     # type = "str"（默认）|"dword"|"bin"|"expand"|"multi_sz"
    name = "Blob", value = "DE AD BE EF",               # view = "default"|"64"|"32"（每指令内联 SetRegView）
    type = "bin", view = "64" },
  { op = "exec_shell", verb = "open",                   # NSIS ExecShell（verb 空 = open）
    target = "https://example.com/readme" },
  { op = "rmdir", path = "$INSTDIR\\legacy",            # NSIS RMDir（默认仅删空目录）
    recursive = true },                                 # recursive = true → RMDir /r 整树删除
  { op = "if_file_exists", path = "$INSTDIR\\old.cfg",  # NSIS IfFileExists（then/else 嵌套指令块）
    then = [
      { op = "delete_file", path = "$INSTDIR\\old.cfg" },
    ], else = [
      { op = "detail_print", text = "全新安装" },
    ] },
  { op = "run", cmd = "$INSTDIR\\setup.bat", args = "/q", abort_on_error = false },
  { op = "abort", message = "配置异常" },               # NSIS Abort（中止安装，message 记原因）
]
# 路径可用 NSIS 常量变量：$INSTDIR $EXEDIR $DESKTOP $SMPROGRAMS $STARTMENU
# $QUICKLAUNCH $APPDATA $LOCALAPPDATA $TEMP $WINDIR $SYSDIR $PROGRAMFILES
# 注意：[[components]] 是数组表，code 必须写内联数组（不能写 [[components."名".code]]）

[[install_types]]             # NSIS InstallTypes / Inno [Types]（可选）
name = "典型"
components = [0]              # 选中该类型时默认勾选的组件索引
default = true                # 默认选中的类型（未标记时取第一个）

[[install_types]]
name = "完整"
components = [0, 1]
# 手动改动任意组件勾选自动切到 Custom（组件页自动附加）

[[registry]]
key = "HKCU\\Software\\Vendor\\App"   # HKCU / HKLM 前缀
values = { InstallPath = "{install_dir}" }
```

## 架构

```
crates/
├── miedit-core         # 包格式（MIEARC2 归档 + MIEDPKG2 包尾 + MIEDUNIN 卸载内嵌）、配置、Windows 工具
├── miedit-compiler     # 打包库（build_package）+ `mis` CLI（`mis build <script.mis>`，含 PE 图标替换 UpdateResourceW）
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
cargo test --workspace    # 214 个单元测试（本轮新增 compression_params 分派映射、ultra 激进档 roundtrip，压缩算法往返扩容到五算法）：归档（含 Deflate/BZip2/LZMA/Brotli/Zstd 压缩算法往返与激进档）/包尾/CRC32/通配符/路径安全/icon 解析/VERSIONINFO 结构回读/文件错误决策与提示信箱/重启替换/安装类型状态机/系统版本门槛/目录统计/组件过滤映射/slug/语言匹配与 LANGID 映射/多语言文案解析链/卸载器文案键集与解析链/卸载语言匹配索引/环境变量解析与 PATH 合并去重/卸载清单新旧兼容/修复入口与修复统计/包尾 v4 与卸载数据新旧格式/安装目录解析与根路径校验/盘符切换空间刷新回归/内置模板色板/CLI 组件参数解析/LANG 参数提取与 NSIS LCID 覆盖/压缩方式别名归一化与分派映射/CRC 后台校验门/MI 脚本语法高亮与脚本 IDE 纯逻辑（查找/替换/字符字节换算/自动缩进/括号补全/命令面板过滤/面包屑/缩进检测）/MI 脚本 v2 组件指令（变量展开/校验/serde 往返/互斥名构造/UAC 执行级别与 manifest XML/CopyFiles 复制内核与自嵌套防护/INI 写入语义/磁盘空间预检判定）
```

已在 Windows 10 x64 实机验证：CLI 与 GUI 构建链路、静默安装/卸载全清理、组件过滤生效、
默认图标链路（未配置自动内嵌）、solid 压缩（592KB→103KB）、升级检测、旧格式兼容。
Linux 无头实机验证（#13）：6 种压缩档端到端构建 + 静默安装（退出码 0、逐字节一致）
+ install.log 落盘；体积排序 lzma-solid < brotli-solid < deflate-solid。

## License

MIT
