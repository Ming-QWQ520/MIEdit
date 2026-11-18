# NSIS 功能对齐路线图

> 目标：MIEdit 功能**完全对齐 NSIS**（以 NSIS 为主，兼顾吸收 Inno Setup 优点）。
> 本文件是持续迭代的作业清单：每次迭代按顺序取下一个"待办"步骤实现，
> 完成后把状态改为"已完成"，并在此登记变更摘要与提交号。
> 设计取舍记录在 [lessons-from-nsis-inno.md](lessons-from-nsis-inno.md)。

## 状态总览

| 步骤 | 内容 | 参考（谁的做法） | 状态 |
| --- | --- | --- | --- |
| 1 | SetOverwrite 覆盖策略 + 源时间戳保留 | NSIS `SetOverwrite` | **已完成**（v0.2.0） |
| 2 | Delete /REBOOTOK 重启删除（卸载） | NSIS `Delete /REBOOTOK` | **已完成**（v0.2.0） |
| 3 | 卸载前主程序运行检测 | NSIS `un.onInit` / Inno CloseApplications | **已完成**（v0.2.0） |
| 4 | QuietUninstallString 注册表值 | NSIS / ARP 惯例 | **已完成**（v0.2.0） |
| 5 | install.log 落盘（LogSet） | NSIS `LogSet on` | **已完成**（v0.2.0） |
| 6 | 安装器 EXE 版本信息资源（VERSIONINFO） | NSIS `VIProductVersion`/`VIAddVersionKey` | **已完成**（v0.3.0） |
| 7 | 包完整性 CRC32 校验 + `--ncrc` 开关 | NSIS `CRCCheck`（fileform.h firstheader CRC） | **已完成**（v0.4.0） |
| 8 | 覆盖失败重试交互（安装期文件被占用） | NSIS Retry/Ignore/Abort 文件对话框 | **已完成**（v0.5.0） |
| 9 | 安装期被占用文件重启替换 + "需要重启"完成页 | NSIS `SetRebootFlag` / Inno `restartreplace` | **已完成**（v0.5.0） |
| 10 | InstallTypes 安装类型（Typical/Full/Minimal/Custom） | NSIS `InstallTypes` / Inno `[Types]` | **已完成**（v0.6.0） |
| 11 | 组件页描述侧栏（组件联动描述面板） | NSIS `components.c` 描述框 | **已完成**（v0.6.0） |
| 12 | 最小系统版本门槛 | Inno `MinVersion`/`OnlyBelowVersion` | **已完成**（v0.7.0） |
| 13 | 快捷方式增强：快速启动栏、独立 lnk 任务 | Inno `[Icons]` | **已完成**（v0.7.0） |
| 14 | 卸载完成页统计（文件数/回收空间） | NSIS 卸载统计 | **已完成**（v0.7.0） |
| 15 | 多语言框架（MUI 式语言选择 + 文案包） | NSIS MUI2 | **已完成**（v0.8.0） |
| 16 | zstd 压缩档 | Inno `Compression.Zstd.pas`（等纯 Rust 编码器成熟） | **阻塞**（依赖待成熟，见步骤 16 定义） |
| 17 | ARP ModifyPath/修复入口 | Inno `ModifyPath`/修复向导 | **已完成**（v0.9.0） |
| 18 | 安装/卸载完成详情停留页 | NSIS `MUI_PAGE_INSTFILES`（完成后停留，点「下一步」进 Finish） | **已完成**（v0.10.0） |
| 19 | 卸载器壁纸 | Inno 卸载向导背景图 / NSIS `BGImage` | **已完成**（v0.10.0） |
| 20 | 目录页空间动态刷新 + 安装目录校验 | NSIS DirectoryPage / `AllowRootDirInstall` / `.onVerifyInstDir` | **已完成**（v0.11.0） |
| 21 | 步骤导航 + UI 材质/模板 + Linux/CLI 适配 + 组件空间联动 | HM NIS Edit 向导 / NSIS components 页 / 跨平台打包 | **已完成**（v0.12.0） |
| 22 | 注册表全类型 + 32/64 视图 + 卸载精确清理 | NSIS `WriteRegDWORD/WriteRegBin/WriteRegExpandStr/WriteRegMultiSz/SetRegView` | **已完成**（v0.13.0） |
| 23 | 安装前/后 + 卸载前自定义命令（ExecWait） | NSIS `Exec`/`ExecWait` / Inno `[Run]` 前后段 | **已完成**（v0.13.0） |
| 24 | 组件分组树（SectionGroup） | NSIS `SectionGroup`/`SectionGroupEnd` | **已完成**（v0.13.0） |
| 25 | 压缩格式扩充：LZMA/LZMA-solid/Brotli/Brotli-solid + 版本号固定 0.1.0 | NSIS `SetCompressor` zlib/lzma(+solid) / Inno 压缩可插拔 | **已完成**（0.1.0，#13） |
| 26 | 卸载器多语言（语言选择页 + 全部文案随语言切换） | NSIS `MUI_UNPAGE_LANGUAGE` / un 语言链 | **已完成**（0.1.0，#14） |
| 27 | 环境变量/PATH 管理（写入 + 广播 + 卸载精确清理） | NSIS WriteEnvStr 惯例 / Inno `ChangesEnvironment` | **已完成**（0.1.0，#14） |
| 28 | 卸载语言跟随安装 + 卸载清单 NSIS 处理语义 | NSIS un.onInit 读回安装语言 / UninstallLog LIFO + `RMDir` 仅空删除 | **已完成**（0.1.0，#15） |
| 29 | MI 脚本语言（.mi）+ VSCode 式脚本 IDE 页 + HM Edit 式导航栏 | HM NIS Edit 向导树 / VSCode 编辑器（行号+语法高亮+校验状态栏） | **已完成**（0.1.0，#16） |
| 30 | 安装包打开速度优化（CRC/壁纸后台化） | NSIS CRCCheck 语义保留、窗口先开（体验优先） | **已完成**（0.1.0，#16） |
| 31 | 全仓 warning 清零（float fallback/未用符号/依赖归位） | rustc `float_literal_f32_fallback`（未来硬错误） | **已完成**（0.1.0，#16） |
| 32 | 脚本编辑器深度对齐 VSCode（查找替换/命令面板/转到行/minimap/自动缩进/括号补全/状态栏/面包屑） | VSCode `editor.find`、`Ctrl+Shift+P`、`Alt+Z`、minimap、Dark+ 配色 | **已完成**（0.1.0，#17） |
| 33 | MI 脚本 v2 指令层：组件级自定义指令 + NSIS 常量变量展开 | NSIS Section 指令语义（Exec/WriteReg/CreateDirectory/Delete/DetailPrint）+ `$INSTDIR` 等 shell 常量 | **已完成**（0.1.0，#18） |
| 34 | 安装器/卸载器单实例互斥 | NSIS CreateMutex 惯例 / Inno `AppMutex` | **已完成**（0.1.0，#18） |
| 35 | `/LANG=` 命令行语言覆盖 | NSIS `/LANG=<十进制 LCID>` | **已完成**（0.1.0，#18） |
| 36 | UAC 执行级别清单（RequestExecutionLevel + supportedOS/DPI/长路径） | NSIS `RequestExecutionLevel` / `ManifestDPIAware` | **已完成**（0.1.0，#19） |
| 37 | MI 指令扩充：Rename/CopyFiles/WriteINIStr/Sleep | NSIS `Rename`/`CopyFiles`/`WriteINIStr`/`Sleep` | **已完成**（0.1.0，#19） |
| 38 | 安装期磁盘空间预检（覆盖抵扣 + 余量） | NSIS 磁盘满中止语义（Error writing to file） | **已完成**（0.1.0，#19） |

## 迭代步骤定义

### 步骤 6：安装器 EXE 版本信息资源（NSIS VIProductVersion）
- **做什么**：编译器在成品安装包 EXE 与卸载器 EXE 上嵌入 VERSIONINFO 资源
  （ProductName / ProductVersion / FileVersion / CompanyName / FileDescription / LegalCopyright），
  数据来自 `[app]`（name/version/publisher）；复用 `crates/miedit-compiler/src/icon.rs`
  已有的 `UpdateResourceW` 链路（`BeginUpdateResourceW` → `UpdateResourceW(RT_VERSION)`）。
  需要手工构造 VS_VERSIONINFO 结构体字节序列（16 位对齐）。
- **验收**：Windows 资源管理器 → 属性 → 详细信息能看到版本字段；`cargo check --workspace` 通过。
- **回归注意**：图标替换与 VERSIONINFO 替换在同一次资源更新中完成，注意先改图标再改版本或合并事务。

### 步骤 7：包 CRC32 校验 + `--ncrc`
- **做什么**：Trailer 增加各分区 CRC32（兼容方案：新增 `MIEDPKG3` magic + 旧格式回退，
  或在 Trailer 后追加校验块）；安装器启动时校验 payload/config/uninstaller 分区，
  失败弹出"安装包已损坏"错误；`--ncrc` 跳过（NSIS CRCCheck off/force 语义）。
- **验收**：篡改包内一个字节后安装器报错；`--ncrc` 跳过；旧格式包仍可安装。
- **参考**：NSIS `Source/exehead/fileform.h`（firstheader 的 length_of_header/CRC 字段）。

### 步骤 8：覆盖失败重试交互（NSIS 文件对话框）
- **做什么**：安装写文件失败（`overwrite = on` 且文件被占用）时，
  向导内弹出"重试 / 忽略 / 中止"三选（egui 模态卡片），
  重试循环、忽略则记录日志并继续、中止则回滚提示。
- **验收**：锁定目标文件后安装，出现交互且三条路径行为正确。

### 步骤 9：安装期重启替换 + "需要重启"完成页
- **做什么**：写文件失败且用户选择"忽略"或 `overwrite = try` 失败时，
  调用 `winutil::schedule_replace_on_reboot`（先 MoveFileEx 把旧文件改名为 `.miedel`，再 MoveFileEx 拷贝目标）
  记录待重启项；完成页显示"需要重启以完成安装"琥珀提示；静默模式记录日志。
- **参考**：NSIS `SetRebootFlag` / Inno `RestartReplace`。

### 步骤 10：InstallTypes 安装类型
- **做什么**：`[[install_types]]`（name + 默认勾选的组件规则集合），
  组件页顶部单选（Typical/Full/Minimal/Custom），切换类型联动组件勾选；
  手动改动任意组件自动切到 Custom（NSIS 语义）。
- **参考**：NSIS `SectionSetFlags`/InstallTypes，Inno `[Types]`。
- **验收**：类型切换 → 组件勾选正确联动；Custom 高亮正确。

### 步骤 11：组件页描述侧栏
- **做什么**：组件页下方（或右侧）描述面板，跟随选中组件显示 `description`
  （NSIS 组件页描述框语义；必装组件显示"必需"徽标）。
- **验收**：点击组件行描述面板更新；空描述显示占位。

### 步骤 12：最小系统版本门槛
- **做什么**：`install.min_version = "6.1"`（Windows 7+）；
  安装器启动时用 `RtlGetVersion`（ntdll）取真实版本，低于门槛报错退出（NSIS/Inno 语义）。
- **验收**：低版本模拟测试（可配置门槛为高版本验证拒绝路径）。

### 步骤 13：快捷方式增强
- **做什么**：Inno `[Icons]` 风格 `[[shortcuts.entries]]`：自定义名称/子目录/参数/图标/工作目录，
  支持快速启动栏；保留现有 `start_menu/desktop` 简单开关的兼容。
- **验收**：demo 脚本添加条目后，安装产物包含全部 lnk 且卸载全部清除。

### 步骤 14：卸载完成页统计
- **做什么**：卸载器统计删除文件数、目录数、释放字节数（清单 size 汇总或逐文件统计），
  完成页展示（NSIS 卸载完成统计语义）。

### 步骤 15：多语言框架（MUI 式）
- **做什么**：`[[ui.languages]]` 语言包（文案键值表）+ 欢迎页前语言选择（或按系统语言自动选），
  默认内置 zh-CN 与 en-US 两包；`ui.text` 仍可单键覆盖。
- **参考**：NSIS MUI2 `!insertmacro MUI_LANGUAGE`。

### 步骤 16：zstd 压缩档
- **做什么**：待纯 Rust zstd 编码器成熟后接入（当前 lzma-rs 编码器为存根，见 lessons 文档）；
  `install.compression = "zstd"`，Installer 解压对应支持。
- **阻塞说明**（2026-11-16 记录）：纯 Rust 生态中 `ruzstd` 仅有**解码器**，
  无成熟 zstd 编码器实现；官方 `zstd` crate 是 C 绑定（cc 构建 libzstd），
  违反工程约束 1（不引入 C 构建链）。待纯 Rust 编码器（如 zstd-rs 类项目）
  发布稳定版本后再接入。转向步骤 17 继续。
- **补充**（2026-11-18，#13）：① LZMA 侧已用 **lzma-rust 0.1.7**（tukaani
  xz-for-java 纯 Rust 移植，sevenz-rust 同源，自带 encoder）解阻，
  `lzma`/`lzma-solid` 已落地（步骤 25）；② **bzip2 同样阻塞**——
  `bzip2-rs` 仅有解码器，`bzip2` crate 是 C 绑定；③ zstd 维持阻塞。
  NSIS 三档中 zlib/lzma 已纯 Rust 对齐，仅 bzip2 待生态成熟。

### 步骤 17：ARP ModifyPath/修复入口（已实现，v0.9.0）
- **设计**：卸载注册表键写 `ModifyPath = "<安装器路径>" --repair`；安装器解析
  修复开关后跳过全部向导页直达修复确认页（含修复摘要统计），确认后
  全量重装文件 + 重建快捷方式/注册表，完成页切换修复文案。
- **实现**：`miedit_core::repair`（modify_path_value / repair_selection /
  summarize）+ installer `--repair`/`/repair` 解析 + `Page::RepairConfirm`
  + finalize 注册表写入；静默修复等同静默安装。

### 步骤 18：安装/卸载完成详情停留页（NSIS INSTFILES 完成态）
- **做什么**：NSIS MUI2 的 INSTFILES 页在安装/卸载完成后**停留**在该页
  （显示完成状态 + 全量日志），用户点「下一步」才进入 Finish 页；
  此前 MIEdit 完成后直接跳完成页、跳过详情。
- **实现**：安装器 `install_finished` 状态（poll_state 不再自动切页，
  Install 页渲染完成横幅 + 「下一步」按钮 → Finish，Enter 放宽）；
  卸载器新增 `Phase::Summary` 详情停留页（完成徽标 + 统计 + 全量日志
  默认展开，「下一步」→ Done 总结页）。

### 步骤 19：卸载器壁纸
- **做什么**：卸载向导支持脚本配置背景图（对齐安装器 `ui.background`）。
- **实现**：`install.uninstall_background`（源文件路径）→ 编译器读图
  写入包尾新分区（**Trailer v4 `MIEDPKG4`**：v3 超集 + 卸载器壁纸
  分区，132B；读取兼容 v2/v3）→ 安装器 finalize 把壁纸随清单/图标
  内嵌进卸载器 EXE 尾部（**MIEDUNIN 32B footer**：+bg_len，读取兼容
  旧 24B footer）→ 卸载器解码贴图（全窗铺满 + 半透明遮罩保证可读，
  窗口自动放大 800×540）；无壁纸走内置渐变（旧包/缺省行为不变）。

### 步骤 20：目录页空间动态刷新 + 安装目录校验（NSIS DirectoryPage 对齐）
- **缺陷背景**（用户报告）：切换盘符后剩余空间不刷新，一直显示默认
  安装盘的可用空间——`disk_free()` 基于仅在 `start_install` 时才更新的
  `dest`，而非目录页实时编辑的 `dir_text`。
- **做什么**：① 可用空间与空间不足禁用逻辑改为基于**当前输入目录**
  所在盘实时查询（NSIS DirectoryPage：Space available 随所选驱动器
  刷新）；② 目录校验：空/相对路径/非法字符/**不存在盘符**拒绝前进并
  提示（NSIS 对无效路径的处理）；③ `install.allow_root_dir_install`
  （NSIS `AllowRootDirInstall`，默认 false = 禁止盘根/UNC 共享根安装）。
- **实现**：纯函数下沉 `miedit_core::installdir`（`parse_dir_text`
  清洗引号/空白/裸盘符规范化、`drive_root_of` 盘根与 UNC 共享根提取、
  `is_root_path`、`classify_inst_dir` 三态校验 + 文案键映射）9 项单测；
  安装器 `current_dest()`/`dir_issue()`/`dir_next_enabled()` 统一三处
  口径（空间显示/Enter 校验/按钮禁用）；`disk_free_space` 无已存在
  祖先目录时返回 None（不再误回退 C 盘）；文案新增 `dir_invalid`/
  `dir_root_warning` 双语键。向后兼容：`allow_root_dir_install` 为
  serde default 字段，旧脚本行为仅受 NSIS 安全默认影响（旧包内置旧
  安装器，行为不变）。
- **验收**：目录页切换盘符可用空间立即变化；输入不存在盘符/非法字符
  时「安装」禁用并提示；默认根目录被拒绝、开关打开后放行。

### 步骤 21：安装导航 + UI 材质/模板 + Linux/CLI 适配 + 组件空间联动
- **做什么**（七项，用户直接指令驱动）：
  ① 盘符切换空间即时刷新回归强化（`D:/Demo` → `E:/Demo` 场景单测，
  正/反斜杠链路全程断言）；
  ② **安装导航对齐 HM NIS Edit**：左列步骤导航栏（当前步 accent 高亮、
  已完成勾点、未到空心点；仅指示不可点击——NSIS 页面流线性），
  `ui.show_step_rail` 开关（默认 true）；
  ③ **Linux 适配**：路径模板 Linux 实现（`{autopf}`→`/opt`、
  `{appdata}`/`{localappdata}`→XDG 数据目录、分隔符归一，Windows
  风格脚本跨平台可用）；快捷方式 = freedesktop `.desktop`（start_menu
  → applications 目录、desktop → 桌面）；is_admin = euid==0；安装器
  字体回退链加入 Noto CJK/文泉驿；finalize 在 Linux 落盘
  `uninstall.json`（卸载器自带清单文件回退读取，PE 卸载器不写出）；
  per_machine 需 root（sudo 提示）；CI 增加 ubuntu job 产出
  `MIEdit-linux.zip`（installer + uninstaller）；
  ④ **命令行安装适配**：`--help`/`--version`/`--components=<a,b>`
  （Inno /COMPONENTS 语义，组件名/1 起始序号，GUI 与静默同生效）/
  `--no-shortcuts`；退出码规范 0/1/2（成功/失败/参数无效）；
  ⑤ **液态玻璃/亚克力材质**：`ui.material = "glass"`（默认）半透明
  卡片 + 高光描边 + accent 主按钮微透明；`ui.acrylic`（默认 false，
  Win11 22H2+ DWM SystemBackdrop，失败平台自动回退绘制玻璃）；
  ⑥ **五套安装模板**：`ui.template` = aurora / midnight / sunrise /
  forest / mono（模板整体接管调色板，优先于 ui.* 显式色）；
  ⑦ **组件页 Space required 联动**：空间需求随组件勾选实时刷新
  （NSIS components page 语义，带文案标签）。
- **实现**：纯函数下沉 `miedit_core::templates`（5 模板 + apply +
  5 单测）、`miedit_core::cliargs`（components 解析 + 退出码 + 5 单测）、
  `installtypes::selection_to_rules`（组件勾选→规则集统一口径 + 1 单测）；
  安装器 rail/材质/模板渲染 + finalize Linux 分支 + main CLI 全量；
  文案新增 6 个 nav_* 键（双语同步，键集一致性测试守护）。
- **验收**：core 单测全过（87 项）；Linux/Windows 双平台 check 0 error；
  YAML 语法校验通过。

### 步骤 22：注册表全类型 + 32/64 视图 + 卸载精确清理（NSIS SetRegView 对齐）
- **做什么**：① `[[registry]]` 值类型从仅 REG_SZ 扩展到全类型：`dwords`
  （十进制/`0x` 十六进制/负数 32 位回绕）、`binary`（十六进制串，长度偶数）、
  `expand_sz`、`multi_sz`，加 `view = "default/64/32"`（NSIS `SetRegView`，
  KEY_WOW64_32KEY/64KEY）；② 卸载精确清理：安装时记录键是否为本次新建
  （`registry_key_exists` 判断），卸载时**新建键整树删除、既有键只删本次写入的
  值并尝试清理空壳键**——不再误伤安装前已存在的内容（旧清单无
  `registry_values` 字段 → 回退整树删除，行为不变）。
- **实现**：解析/校验纯函数下沉 `miedit_core::regspec`（parse_dword/
  parse_hex_binary/parse_view/resolve_entries，6 项单测）；winreg 0.52 无
  REG_BINARY/REG_EXPAND_SZ 的 `ToRegValue` 映射，用 `set_raw_value` 手工构造；
  编译器新增 5.2.2 注册表项校验（与安装期同一套纯函数）。
- **验收**：全类型往返单测；双平台 check 0 error；旧清单/旧脚本行为不变。

### 步骤 23：安装前/后 + 卸载前自定义命令（NSIS Exec/ExecWait）
- **做什么**：`[[install.exec_before]]`（解包前，NSIS `.onInit`/`Exec`）、
  `[[install.exec_after]]`（清单/卸载器写入完成后，ExecWait）、
  `[[install.exec_before_uninstall]]`（卸载删除阶段前，如停服务/关进程）；
  每条 `cmd/args/working_dir`（`{install_dir}` 模板）+ `abort_on_error`
  （退出码非 0 中止）+ `optional`（程序缺失仅告警）；GUI 与静默同生效；
  Windows 子进程 CREATE_NO_WINDOW 不闪黑框；退出码/输出尾部写入日志。
- **实现**：展开/切分/决策纯函数下沉 `miedit_core::runcmd`
  （split_args 引号感知、should_abort 矩阵、log_lines，4 项单测）；
  卸载前命令由安装器复制进卸载清单（卸载器无脚本只读清单）；
  卸载命令失败不中止卸载（占用文件走重试/重启删除路径）。
- **验收**：决策矩阵单测；双平台 check 0 error；编译器拒绝空 cmd。

### 步骤 24：组件分组树（NSIS SectionGroup）
- **做什么**：`[[components]]` 新增 `group = "父组/子组"`（`/` 分隔多级，
  空 = 根级，TOML 友好且等价于 NSIS SectionGroup/SectionGroupEnd 嵌套）；
  组件页树形渲染：组行可折叠/展开（▸/▾）、组复选框三态（全选/部分/全不选）
  联动子组件（必装组件保持勾选）；声明顺序保持、中间组隐式创建；
  安装类型与 CLI `--components=` 仍按扁平索引运作。
- **实现**：树构建/组勾选/三态/折叠判定纯函数下沉
  `miedit_core::componenttree`（5 项单测）；安装器组件页改树渲染；
  builder 组件卡片加 group 输入框。
- **验收**：建树/联动/折叠单测；双平台 check 0 error。

### 步骤 25：压缩格式扩充（NSIS SetCompressor 对齐 + 扩展）+ 版本号固定
- **做什么**：用户指令「增加更多压缩格式 + 版本固定为 0.1.0」：
  ① `install.compression` 从 2 档扩到 6 档：`deflate` / `solid`（原有）+
  **`lzma` / `lzma-solid`**（NSIS 旗舰 LZMA，`lzma-rust 0.1.7` 纯 Rust 编解码）+
  **`brotli` / `brotli-solid`**（`brotli` crate 纯 Rust 编解码）；
  ② 归档条目标志位扩展：bit2=brotli、bit3=lzma（bit0/bit1 不变，旧包兼容）；
  ③ 压缩/解压纯函数下沉 `miedit_core::archive`（`Algo` 枚举 +
  `compress_bytes`/`decompress_bytes` + `normalize_compression` 别名归一化），
  编译器与安装器共用，commonly 移除两 crate 对 flate2 的直接依赖；
  ④ builder 压缩下拉 6 选项 + 预览摘要同步；⑤ workspace 版本 0.13.0
  → **0.1.0 固定**（协议第 6 条修订：不再随批次递增）。
- **实现**：LZMA 用 header 自述格式（`LZMAWriter::new_use_header` 编码 ↔
  `LZMAReader::new_mem_limit` 解码），preset 6（8MB 字典）；Brotli quality 9。
  新增 6 项单测（三算法条目级/整包往返、压缩比对照、别名归一化、
  旧标志位兼容锚点）。
- **验收**：core 108 项全过；双平台 check 0 error；Linux 实机 6 档端到端
  （构建 + 静默安装退出码 0 + payload 逐字节一致 + install.log）；
  体积排序 lzma-solid < lzma < brotli-solid < brotli < deflate ≈ solid。

### 步骤 26：卸载器多语言（NSIS un MUI 对齐）
- **做什么**：安装器已有 MUI2 式语言框架（步骤 15），卸载器却是中文硬编码。
  对齐 NSIS 卸载器语言机制：① 卸载清单新增 `languages`（安装器把
  `ui.languages` 随清单内嵌/落盘）；② 清单携带 >1 语言时卸载器先显示
  **语言选择页**（NSIS `MUI_UNPAGE_LANGUAGE`，预选系统语言，确认后同步
  窗口标题）；③ 卸载器全部 UI（确认/进度/详情/完成/失败/按钮）与日志
  （静默/详情页）文案键化（45 个 `un_*` 键，英文默认表 + 内置 zh-CN 包
  双语同步，键集一致性测试守护）；④ 静默模式按系统语言自动预选。
- **向后兼容**：旧清单无 `languages` 字段 → serde default 空 → 解析链回退
  内置 zh-CN（历史中文文案行为不变）；新清单语言解析与安装器同链
  （语言包 text > 内置包 > 英文默认）。
- **顺带修复**：确认页/语言页取消或 Esc 放弃卸载时不再删除清单与自删除
  卸载器（旧版取消后安装再也不能卸载——NSIS 取消语义是保留原状）。
- **验收**：文案键集/解析链/清单新字段往返 + 旧 JSON 兼容单测；Linux 实机
  清单内嵌语言包 + 独立卸载器读取卸载成功。

### 步骤 27：环境变量/PATH 管理（NSIS WriteEnvStr 惯例 / Inno ChangesEnvironment 对齐）
- **做什么**：① `[[install.env]]`（name/value/part/scope）：`part = all`
  直接设值 / `path_append` 追加 / `path_prepend` 前置（`path_*` 为 `;`
  列表语义：ASCII 不区分大小写去重合并，不覆盖已有内容）；scope =
  `user`（HKCU\Environment，默认）/ `machine`（HKLM\...\Session
  Manager\Environment，需管理员，无权限时告警跳过）；② 写入后广播
  `WM_SETTINGCHANGE` "Environment"（新启动进程立即可见，无需注销）；
  ③ 卸载精确清理：`path_*` 合并写入只**移除本安装写入的段**后回写
  （不误伤其他软件的段；列表空且安装前变量不存在则删除整个值）；`all`
  写入安装前已存在则**还原安装前原值**、新建则删除整个值；④ 编译器
  5.2.3 校验（name/value 非空、part/scope 合法、path_* 单段无 `;`、
  同变量同作用域不重复）；⑤ builder 增加环境变量编辑卡片。
- **实现**：拆分/合并/去重/移除/展开纯函数下沉 `miedit_core::envspec`
  （6 项单测）；注册表读写 + 广播在 `winutil::env_read/env_write/
  env_delete/env_broadcast`（`Path` 与含 `%VAR%` 值写 REG_EXPAND_SZ，
  其余 REG_SZ）；清理引用 `EnvCleanupRef` 随清单内嵌/落盘。非 Windows
  平台为桩（同注册表）。
- **验收**：envspec 单测（含 merge→remove 往返还原）；双平台 check
  0 error；Linux 端到端（构建 + 静默安装退出码 0 + 平台桩日志）。

### 步骤 28：卸载语言跟随安装 + 卸载清单 NSIS 处理语义（用户指令 2026-11-18）
- **做什么**：① **卸载语言跟随安装**（NSIS MUI 官方模板惯例：`un.onInit`
  读回安装语言）：卸载清单新增 `language` 字段记录安装时**实际选定**的
  语言 id（安装器 finalize 写入；单语言包安装器跳过语言页同样记录）；
  卸载器按精确 → 主标签匹配语言包直接使用（用户装时选 English →
  卸载界面即 English），不再显示语言选择页；② **卸载清单 NSIS 处理
  语义**：文件按安装**相反顺序删除**（NSIS UninstallLog/LIFO 惯例，最后
  装入的先删，快捷方式同）、目录 **NSIS `RMDir` 语义——仅删除空目录**，
  非空目录保留并记录日志（不算失败，卸载绝不碰清单外内容），缺失项
  静默跳过（`Delete`/`RMDir` 语义，原有行为保留）。
- **向后兼容**：旧清单（#14 及以前）无 `language` 字段 → 空 → 回退步骤
  26 行为（系统语言自动预选；旧清单多语言仍显示语言选择页）；清单
  serde default 兼容，旧包行为不变。
- **实现**：`langs::match_language_index`（精确/主标签匹配，与
  `initial_language` 同口径，纯函数 + 单测）；卸载器 `resolve_preselect`
  统一静默/GUI 预选；目录判定 `dir_exists_and_nonempty`（read_dir 首
  条目探测，read 失败交由 NotFound=Ok 语义）；顺带把目录拼接从字符串
  `+ "\\"` 改为 `Path::join`（Linux 卸载路径修复）。
- **验收**：match_language_index 单测 + 清单 `language` 往返/旧 JSON
  兼容单测；双平台 check 0 error。

### 步骤 29：MI 脚本语言（.mi）+ VSCode 式脚本 IDE 页 + HM Edit 式导航栏（用户指令 2026-11-18）
- **做什么**：① **脚本后缀正式定为 `.mi`**（MI 脚本语言 v1：TOML 语法，像
  NIS 是 NSIS 的脚本一样是 MIEdit 的脚本）——builder 文件对话框、示例
  （`examples/demo-app.mi`）、编译器 CLI 用法、README 全部改名；打开
  兼容旧 `.toml`/`.mris.toml`；② **脚本页升级为 VSCode 式轻量 IDE**：
  文件标签栏（文件名 + 未保存圆点）→ 工具栏（重新生成/应用到向导/
  保存脚本/另存为）→ 编辑器（`TextEdit::layouter` 实现行号 gutter +
  MI 脚本语法高亮：注释绿/节头蓝/键浅蓝/字符串橙/数值紫/布尔蓝，纯
  函数 `mi_script_job` 无第三方依赖）→ 状态栏（行数/字符数/实时校验）
  → 问题面板（解析错误就地显示）；只适配本项目的脚本；③ **导航优化
  （借鉴 HM NIS Edit）**：builder 新增左栏步骤树（全部步骤可点直达，
  当前步高亮、已完成步 √ 徽标），跳转与页脚「下一步」共用 `goto_step`
  （进入脚本页按脏状态重新生成）。
- **验收**：`mi_script_job` 5 项单测（行号/节头/键/字符串/数值/布尔/
  字符串内 `#` 不误判）；双平台 check 0 error。
- **后继（步骤 32 规划）**：MI 脚本 v2 指令式扩展（NSIS Section/指令层，
  如 `Run`/`SectionSet` 钩子），保持 TOML 为基础语法向上叠加。

### 步骤 30：安装包打开速度优化（用户指令 2026-11-18）
- **做什么**：GUI 模式关键路径只保留「包尾解析 + 配置解析 + 图标解码」；
  **CRC32 校验移入后台线程**（`CrcGate`：安装线程在提取任何文件之前
  等待校验结果——损坏包仍然拒绝安装、`--ncrc` 语义不变，只是不再阻塞
  窗口；CRC 失败立即转错误页）；**壁纸解码（IO + PNG）移入后台线程**
  （先渐变后壁纸的渐进式呈现）。静默模式保持同步校验（确定性退出码）。
- **实测**（release，Linux，表驱动 CRC32 ~400MB/s）：8MB 包关键路径
  19ms→0ms；64MB 包 160ms→0ms；**256MB 包 637ms→0ms**（基准测试
  `compiler/tests/startup_bench.rs`，`--ignored` 按需运行）。壁纸 PNG
  解码（数百 ms 级）同步移出。
- **验收**：Linux 实机端到端——干净包静默装 exit 0；篡改 payload 一字节
  → 静默装 exit 1 + 明确报错；`--ncrc` 跳过校验（损坏数据在解压时自然
  失败，NSIS `/NCRC` 同语义）；`CrcGate` 3 项单测。

### 步骤 32：脚本编辑器深度对齐 VSCode（用户指令 2026-11-18「优化编辑页面对齐vscode」）
- **做什么**：新建 `miedit-builder/src/scriptide.rs` 组件库（纯逻辑单测 + 小部件），
  编辑页对齐 VSCode 的编辑体验：① 查找/替换小部件（Ctrl+F / Ctrl+H，命中计数
  `n/N`、↑↓ 逐个跳转、Aa 大小写开关、单个/全部替换，命中高亮 + 当前命中橙色）；
  ② 命令面板（Ctrl+Shift+P，子串 + 子序列模糊过滤，↑↓+Enter 键盘导航）；
  ③ 转到行（Ctrl+G，选中整行并自动滚动）；④ 自动缩进（Enter 继承上一行缩进，
  `{`/`[` 结尾多一级）；⑤ Tab 转空格（对齐到下一缩进倍数）；⑥ 括号/引号自动补全
  与跨越（`( [` `{` `"` 输入后补配对符，重复输入跨越）；⑦ 高亮层（当前行、
  查找命中、错误行红色下划线，基于 galley 字符→几何映射逐字符定位，CJK 安全）；
  ⑧ 当前行行号亮色（gutter 行号高亮）；⑨ minimap 缩略图（按语法类别分色画条、
  光标行 accent 标记、点击跳行）；⑩ 面包屑（`文件 › [当前节]`）；⑪ 状态栏
  （Ln/Col、选中字符数、缩进风格检测、换行开关、UTF-8、语言模式、错误计数）；
  ⑫ 菜单栏（文件/编辑/查看/工具/帮助）+ VSCode 快捷键全套；⑬ 自动换行开关
  （Alt+Z，默认开）；⑭ Dark+ 主题色（#1E1E1E 背景/#6A9955 注释/#B5CEA8 数值等）。
- **纯逻辑单测**：查找（大小写/不重叠/中文偏移）、字符↔字节换算、替换（单个/全部）、
  自动缩进、Tab 转空格、括号补全/跨越、命令面板过滤（子串/子序列）、面包屑、
  缩进检测、toml 错误行列解析、行号宽度增长、行号映射、高亮布局（含当前行
  行号亮色）。builder 测试 5→23 项。
- **验收**：`cargo test --workspace` 全过；双平台 check 0 error 0 warning。

### 步骤 33：MI 脚本 v2 指令层（组件级自定义指令 + NSIS 常量变量展开）
- **做什么**：在 MI 脚本 v1（声明式 TOML）上叠加 **NSIS Section 指令语义**——
  每个组件新增 `code` 指令数组（内联表数组，`op` 判别），组件被勾选后
  按脚本顺序执行，未勾选组件不执行（NSIS Section 语义）：① `run`
  （NSIS `Exec`/`ExecWait`，复用 runcmd 执行器，`abort_on_error` 显式中止）；
  ② `write_reg`（`WriteRegStr`/`WriteRegDWORD`：`type = "str"|"dword"`，
  新建键/既有键判定随清单精确清理）；③ `create_dir`（`CreateDirectory`：
  安装目录内记相对路径，目录外记清单 `extra_dirs` 新字段，卸载按
  `RMDir` 仅空删除语义清理）；④ `delete_file`（`Delete`：缺失静默跳过）；
  ⑤ `detail_print`（`DetailPrint`：详情日志行）。路径支持 NSIS 常量变量
  （`$INSTDIR`/`$EXEDIR`/`$DESKTOP`/`$SMPROGRAMS`/`$STARTMENU`/`$QUICKLAUNCH`/
  `$APPDATA`/`$LOCALAPPDATA`/`$TEMP`/`$WINDIR`/`$SYSDIR`/`$PROGRAMFILES`），
  文件系统路径安装期按平台分隔符归一化（与解包链路同口径；注册表键
  的 `\` 是 hive 分隔符不归一化）。指令失败按 NSIS error-flag 语义记
  警告继续。builder 问题面板新增指令静态校验（必填字段/dword 合法性/
  hive 前缀）。语法要点：`[[components]]` 是 TOML 数组表，`code` 必须
  以内联数组写在组件表内（`[[components."名".code]]` 会变成独立未知子表）。
- **实现**：`miedit-core/src/micmd.rs`（MiInstruction 枚举 + `expand_vars`
  纯函数长名优先 + `MiVars::standard_map` 跨平台 Shell 目录 + `validate` +
  `describe`，8 单测）；config `ComponentSpec.code`（serde default，旧脚本
  兼容）；install.rs `run_component_code`/`execute_instruction`（副作用
  经 `CodeEffects` 合并进清单）；卸载器新增 `extra_dirs` 清理（最深先删、
  仅删空、缺失静默跳过）。builder 新增 toml 解析 + 重新生成往返单测。
- **验收**：Linux 实机端到端——含指令包构建 + 静默安装 exit 0（日志含
  指令行、`$INSTDIR\logs` 归一化落盘、`$TEMP\e2e-extra-dir` 建目录并
  入清单）+ 卸载 exit 0（extra_dirs 精确清理）；全仓 160 项单测全过；
  双平台 check 0 error 0 warning。

### 步骤 34：安装器/卸载器单实例互斥（NSIS CreateMutex 惯例 / Inno AppMutex 对齐）
- **做什么**：安装器启动后（配置校验后、组件解析前）尝试获取
  `Global\MIEDIT-<app_id>` 命名互斥（`CreateMutexW` +
  `ERROR_ALREADY_EXISTS` 判定，`Global\` 前缀覆盖多会话场景）；已有
  实例运行时 GUI 弹消息框、静默写日志，均以退出码 1 退出。卸载器同
  机制（互斥名加 `-uninstall` 后缀与安装器区分）。守卫句柄 Drop 释放
  （进程退出自动释放双保险）。非 Windows 平台恒成功（Linux 适配：
  无对应惯例，行为不变）。
- **实现**：`winutil::single_instance_mutex_name`（纯函数：非法字符
  替换 `_`、空 id 回退 `app`）+ `acquire_single_instance` +
  `SingleInstanceGuard`（windows-rs `CreateMutexW`，需
  `Win32_Security` feature）；installer/uninstaller main 挂点。
- **验收**：互斥名构造 2 项单测（后缀/非法字符/空值回退）；双平台
  check 0 error；Windows 实机双开验证（本沙箱无法实机验证，行为按
  NSIS 惯例实现）。

### 步骤 35：`/LANG=` 命令行语言覆盖（NSIS `/LANG` 语义）
- **做什么**：安装器接受 `--lang=<id>` / `/LANG=<值>`：① NSIS 十进制
  LCID（`1033` → LANGID 0x0409 → `en-US`，复用内置 LANGIDS 表反查）；
  ② `0x` 前缀十六进制 LANGID（`0x0804` → `zh-CN`）；③ 直接语言 id
  （`zh-CN` / `en`）。GUI 与静默模式同生效，优先级高于系统自动预选/
  语言页预选；无效值宽容回退自动预选（NSIS 同语义，仅静默日志记录）。
  卸载器同样接受 `/LANG=`，优先于清单记录语言（无效值回退清单预选）。
  `--help` 补充参数说明。
- **实现**：`cliargs::parse_lang_arg`（大小写不敏感前缀提取，1 单测）+
  `langs::resolve_lang_override`（LCID→LANGIDS 反查→match_language_index，
  1 单测）；installer main + uninstaller main 挂点；app.rs `run_gui`
  新增 `lang_override` 参数（`.or(initial_language)`）。
- **验收**：解析/匹配单测 2 项；双平台 check 0 error。

### 步骤 36：UAC 执行级别清单（NSIS `RequestExecutionLevel` 对齐）
- **做什么**：编译器在安装器/卸载器 EXE 嵌入 application manifest
  （RT_MANIFEST 资源，类型 ID 24）：`requestedExecutionLevel` 按
  `install.request_execution_level`（`user`/`highest`/`admin`，NSIS
  关键字同款；缺省按 `per_machine` 推导——全机 → admin、按用户 →
  user，取 MIEdit 安装器需要写 Program Files/HKLM 的现实，不采用
  NSIS 的 `none` 空清单）。同一清单声明 supportedOS 五档 GUID
  （Vista…Win11，微软文档标准值，保障 `RtlGetVersion` 真实报告——
  服务步骤 12 版本门槛）与 DPI 感知（`dpiAware=true/pm`）+
  长路径感知（`longPathAware`，NSIS `ManifestDPIAware` 惯例延伸）。
  非法级别构建期报错（NSIS 同语义）。
- **实现**：`miedit_core::uac`（`ExecLevel`/`parse_level`/`manifest_xml`
  纯函数 + 6 单测）；`config.install.request_execution_level`
  （serde default None，旧脚本兼容）；`patch_pe_resources` 增参
  `manifest`（同事务写入 RT_MANIFEST，`make_int_res(24)`）；
  编译器 lib.rs 6.6 段构造清单字节、卸载器/安装器两处调用传入。
- **验收**：级别归一/推导/非法报错/XML 内容（level 属性、五 GUID、
  dpiAware/longPathAware）6 项单测；双平台 check 0 error 0 warning；
  Windows 实机 UAC 弹窗/属性清单验证待实机。

### 步骤 37：MI 指令扩充（NSIS Rename/CopyFiles/WriteINIStr/Sleep）
- **做什么**：组件 `code` 指令集扩至 9 条：`rename`（同盘重命名/
  移动，目标已存在则失败——NSIS 同语义）/ `copy_files`（文件或整
  目录树递归复制，同名覆盖，源不存在报错，目标在源内部拒绝自嵌套）/
  `write_ini`（INI 节键写入，节/键匹配不区分大小写——Windows
  `WritePrivateProfileString` 语义，文件缺失则新建，既有内容原样保留）/
  `sleep`（毫秒暂停）。路径均支持 NSIS 常量变量与平台分隔符归一化。
- **实现**：`micmd.rs` 四新变体 + `validate`（必填/同名/自嵌套检查）
  + `describe`（日志文案）+ `write_ini_str` 纯函数（5 单测）;
  `files.rs::copy_tree`（递归复制内核 + 自嵌套防护，2 单测）；
  installer `execute_instruction` 四运行臂（变量展开/分隔符归一化/
  日志语义与既有指令一致）。
- **验收**：micmd 5 项 + files 2 项新单测；Linux 实机端到端：
  write_ini 新建节/追加节、copy_files 树复制、rename 重命名、
  sleep/detail_print 落日志，静默安装 exit 0 + 卸载 exit 0；
  双平台 check 0 error 0 warning。

### 步骤 38：安装期磁盘空间预检（NSIS 磁盘满中止语义）
- **做什么**：安装写盘前逐文件预检目标盘剩余空间：
  `free + 旧文件占用 < 新文件大小 + 8MB 余量` 时中止安装
  （覆盖写可抵扣旧文件占位，余量防簇分配/日志零头）；错误文案含
  期望/实际/路径（NSIS "Error writing to file / disk full" 信息量
  对齐），进失败页/静默日志，退出码 1。剩余空间不可知时跳过检查
  不误报（NSIS 也是写失败才报，预检更保守）。
- **实现**：`files::SPACE_RESERVE`/`is_space_insufficient`
  （saturating 防溢出）/`space_error_text`（1 单测覆盖充足/不足/
  边界/抵扣/溢出）；installer 解包循环写入前挂点
  （`winutil::disk_free_space` 取目标盘）。
- **验收**：判定边界单测；双平台 check 0 error 0 warning；
  磁盘满实机验证待做（沙箱无法真实写满目标盘）。

1. **纯 Rust**：不引入 C 构建链；新增依赖须在 Windows/Linux 双平台可编译。
2. **向后兼容**：旧格式安装包/卸载清单必须仍可处理（serde default / magic 回退）。
3. **测试**：核心逻辑写成 `miedit-core` 纯函数并加单元测试；`cargo test -p miedit-core`
   在无 Windows 环境下也必须通过；UI 交互逻辑以 `cargo check --workspace` 保证可编译。
4. **小步提交**：一次迭代一个步骤，提交信息格式
   `feat: NSIS 对齐 #N — <内容>`（作者：Ming-QWQ520 <192413597+Ming-QWQ520@users.noreply.github.com>）。
5. **文档同步**：完成步骤后更新本文件状态表、README 特性表与 lessons 文档。
6. **版本号**：**固定为 0.1.0**（用户指令 2026-11-18，自 #13 批次起生效，
   不再随功能对齐批次递增；历史批次 v0.2.0–v0.13.0 保留原版本号记录）。

## 变更日志

- 0.1.0（步骤 36–38，#19）：**UAC 执行级别清单 + MI 指令扩充 +
  安装期磁盘空间预检**（对齐 NSIS `RequestExecutionLevel` /
  `Rename`/`CopyFiles`/`WriteINIStr`/`Sleep` / 磁盘满中止语义）：
  ① **UAC 执行级别清单**（步骤 36）：`install.request_execution_level`
  （`user`/`highest`/`admin`，缺省按 `per_machine` 推导 admin/user），
  编译器在安装器/卸载器 EXE 同事务嵌入 RT_MANIFEST（类型 ID 24）——
  `requestedExecutionLevel`（asInvoker/highestAvailable/
  requireAdministrator）+ supportedOS 五档标准 GUID（Vista…Win11，
  微软文档核实，保障 `RtlGetVersion` 真实版本报告）+ `dpiAware=true/pm`
  + `longPathAware`（NSIS `ManifestDPIAware` 惯例延伸）；非法级别
  构建期报错（NSIS 同语义）；`miedit-core/src/uac.rs` 纯函数
  （parse_level 归一/推导 + manifest_xml，6 单测），`patch_pe_resources`
  增 manifest 参数（cfg(windows)/非 Windows 签名同步）。② **MI 指令
  扩充**（步骤 37，指令集 5→9 条）：`rename`（Rename，目标已存在失败）/
  `copy_files`（CopyFiles，`files::copy_tree` 递归复制 + 自嵌套拒绝）/
  `write_ini`（WriteINIStr，`micmd::write_ini_str` 纯函数——节/键
  不区分大小写、键原位替换、节内末尾插入、缺失节末尾追加、缺失文件
  新建，既有内容原样保留）/ `sleep`（Sleep 毫秒暂停）；validate 新增
  必填/同名/自嵌套检查，describe 对应文案，安装器四运行臂（变量展开 +
  平台分隔符归一化，与既有指令同日志语义），micmd 5 单测 + files 2 单测。
  ③ **安装期磁盘空间预检**（步骤 38）：解包循环写盘前逐文件检查
  `free + 旧文件占用 < 新文件大小 + 8MB 余量` → 中止（覆盖写抵扣
  旧文件占位；saturating 防溢出；剩余空间不可知跳过不误报），
  错误文案含期望/实际/路径进失败页/静默日志（`files::is_space_insufficient`
  + `space_error_text`，1 单测覆盖充足/不足/边界/抵扣/溢出）。
  `examples/demo-app.mi` 同步 `request_execution_level` 与新指令注释。
  Linux 实机端到端：新指令包构建（UAC 级别 user）→ 静默安装 exit 0 →
  write_ini 新建节/追加节/树复制/重命名/日志逐项核对 → 卸载 exit 0
  （指令创建文件不自动回收，NSIS 同语义）。全仓 146→166 项单测全过；
  双平台 check 0 error 0 warning。

- 0.1.0（步骤 33–35，#18）：**MI 脚本 v2 指令层 + 单实例互斥 + /LANG=
  语言覆盖**（对齐 NSIS Section 指令语义 / CreateMutex 惯例 / `/LANG`）：
  ① **MI 脚本 v2 指令层**（步骤 33）：组件新增 `code` 内联指令数组——
  组件勾选后按脚本顺序执行，未勾选不执行（NSIS Section 语义）；指令集
  `run`（Exec/ExecWait，abort_on_error 显式中止）/ `write_reg`
  （WriteRegStr/WriteRegDWORD，新建键/既有键随清单精确清理）/
  `create_dir`（CreateDirectory，目录外记清单 `extra_dirs` 新字段）/
  `delete_file`（Delete，缺失静默跳过）/ `detail_print`（DetailPrint）；
  路径支持 12 个 NSIS 常量变量（`$INSTDIR`/`$DESKTOP`/`$SMPROGRAMS`/
  `$APPDATA`/`$TEMP`/`$WINDIR`/`$SYSDIR`/`$PROGRAMFILES` 等），
  文件系统路径按平台分隔符归一化（Linux 实机适配；注册表键 hive
  分隔符不归一化）；指令失败按 NSIS error-flag 记警告继续；
  `miedit-core/src/micmd.rs`（MiInstruction + expand_vars 长名优先 +
  MiVars::standard_map + validate + describe，8 单测）；builder 问题
  面板新增指令静态校验 + toml 往返单测。语法要点：`[[components]]`
  是数组表，`code` 必须以内联表数组写在组件表内。② **单实例互斥**
  （步骤 34，NSIS CreateMutex 惯例 / Inno AppMutex 对齐）：安装器/
  卸载器 `Global\MIEDIT-<app_id>[-uninstall]` 命名互斥
  （CreateMutexW + ERROR_ALREADY_EXISTS，`Win32_Security` feature，
  Drop 释放双保险），已有实例 GUI 弹框/静默日志 + exit 1；
  非 Windows 恒成功（Linux 行为不变）。③ **`/LANG=` 语言覆盖**
  （步骤 35，NSIS `/LANG=<十进制 LCID>` 语义）：安装器/卸载器
  `--lang=` / `/LANG=` 支持 NSIS 十进制 LCID（1033→en-US）/
  `0x` 十六进制 LANGID / 语言 id 三种写法，GUI+静默同生效，
  优先于自动预选与清单记录语言；无效值宽容回退（NSIS 同语义）；
  `--help` 同步。新增 micmd/winutil/cliargs/langs/builder 共 12 项
  单测，全仓 148→160 项全过；Linux 实机端到端（含指令包构建 +
  静默安装 exit 0 + 指令执行/路径归一化/清单 extra_dirs 落盘 +
  卸载 exit 0 + extra_dirs 精确清理）；双平台 check 0 error 0 warning。

- 0.1.0（步骤 32，#17）：**脚本编辑器深度对齐 VSCode**（用户指令
  2026-11-18「优化编辑页面对齐vscode」）：新建 `miedit-builder/src/scriptide.rs`
  组件库（纯逻辑单测 + 小部件，app.rs 只驱动 IdeState/IdeAction）：
  ① 查找/替换小部件（Ctrl+F / Ctrl+H：命中计数 n/N、↑↓ 逐个跳转、Aa 大小写
  开关、单个/全部替换；命中灰蓝底、当前命中橙底，输入即跳第一个命中）；
  ② 命令面板（Ctrl+Shift+P：子串 + 子序列模糊过滤、↑↓ Enter 键盘导航，
  覆盖保存/另存/查找/替换/转到行/换行/应用/重新生成/构建页跳转等 11 条命令）；
  ③ 转到行（Ctrl+G，选中整行 + 自动滚动）；④ 自动缩进（Enter 继承上一行缩进，
  `{`/`[` 结尾多一级）；⑤ Tab 转空格（对齐到下一缩进倍数）；⑥ 括号/引号自动
  补全与跨越；⑦ 高亮层：当前行高亮、查找命中高亮、错误行红色下划线（基于
  galley 字符→几何映射逐字符定位，CJK 安全，行号前缀偏移精确换算）；
  ⑧ 当前行行号亮色；⑨ minimap 缩略图（语法类别分色画条、光标行 accent 标记、
  点击跳行）；⑩ 面包屑 `文件 › [当前节]`；⑪ VSCode 式状态栏（Ln/Col、选中
  字符数、缩进风格检测、换行开关、UTF-8、语言模式、错误计数）；⑫ 菜单栏
  （文件/编辑/查看/工具/帮助）+ 全套 VSCode 快捷键；⑬ Alt+Z 自动换行开关；
  ⑭ Dark+ 主题色。Esc 分层关闭（面板→转到行→查找）。builder 测试 5→23 项，
  全仓 148 项单测全过；双平台 check 0 error 0 warning。

- 0.1.0（步骤 29–31，#16）：**MI 脚本（.mi）+ VSCode 式脚本 IDE + 打开
  速度优化 + warning 清零**（用户指令 2026-11-18：修复 warning / 脚本
  编辑页像 VSCode 轻型 IDE、脚本后缀改 .mi 自研轻量脚本语言 / 优化编译
  后 exe 安装包打开速度）：
  ① **warning 清零**（步骤 31）：46 处 `Stroke::new` 浮点字面量
  `float_literal_f32_fallback`（rustc 1.100 起为 future-hard-error）全部
  显式 `_f32`；`ENV_KEY_MACHINE` 补 `#[cfg(windows)]`；`serde_json` 移入
  core `[dev-dependencies]`（仅测试使用）；regspec/versioninfo 测试未用
  导入/变量清理。build + test 双 0 warning。
  ② **MI 脚本语言（.mi）+ VSCode 式脚本 IDE 页 + HM Edit 式导航栏**
  （步骤 29）：脚本后缀正式定为 `.mi`（TOML 语法，示例/CLI/文档同步；
  打开兼容旧 .toml）；脚本页重构为轻量 IDE——文件标签栏（未保存圆点）、
  工具栏（重新生成/应用/保存/另存为）、编辑器（`TextEdit::layouter`
  实现**行号 + MI 脚本语法高亮**：注释/节头/键/字符串/数值/布尔分色，
  纯函数 `mi_script_job` 零依赖）、状态栏（行数/字符数/实时校验）、
  问题面板（解析错误就地显示）；builder 新增 **HM NIS Edit 式左栏步骤
  树**（全部步骤可点直达、当前高亮、完成 √，`goto_step` 与页脚共用）。
  新增 5 项 `mi_script_job` 单测。
  ③ **安装包打开速度优化**（步骤 30）：GUI 关键路径只留包尾+配置+图标；
  CRC32 校验移后台线程（`CrcGate`：提取前等待，损坏包仍拒绝安装、
  `--ncrc` 不变、失败立即转错误页）；壁纸解码移后台（先渐变后壁纸）。
  release 实测 256MB 包关键路径 **637ms→0ms**、64MB 160ms→0ms；
  Linux 端到端：干净包 exit 0 / 篡改 payload exit 1 报 CRC 错误 /
  `--ncrc` 跳过校验；新增 `CrcGate` 3 项单测。全仓 130 项单测全过；
  双平台 check 0 error 0 warning。

- 0.1.0（步骤 28，#15）：**卸载语言跟随安装 + 卸载清单 NSIS 处理语义**
  （用户指令 2026-11-18：① 卸载器跟随安装时的语言——只选一个语言直接
  使用；两种语言时以安装时选定的为准，装、卸界面同语言；② 卸载清单
  仿照 NSIS 处理）：
  ① **卸载语言跟随安装**（NSIS MUI 官方模板 `un.onInit` 读回安装语言
  惯例）：清单新增 `language` 字段（安装时选定的语言 id，安装器
  finalize 写入；单语言包 `initial_language` 预选 `Some(0)` 同样记录）;
  卸载器 `resolve_preselect` 按「清单记录 → 精确/主标签匹配
  （`langs::match_language_index`，与安装器同口径）→ 回退系统语言自动
  预选」解析，静默/GUI 共用；语言选择页仅**旧清单**（未记录语言）
  多语言时回退显示。旧清单 serde default 空 → 行为不变（向后兼容）。
  ② **卸载清单 NSIS 处理语义**（UninstallLog/LIFO 惯例）：文件与快捷
  方式按安装**相反顺序删除**；目录 **`RMDir` 语义——仅删除空目录**，
  非空（用户放置了文件/清单外残留）保留并记录日志 `un_log_dir_kept`
  （46 个文案键，不算失败）；缺失项静默跳过（`Delete` 语义，原有）;
  顺带修复目录拼接为 `Path::join`（Linux 卸载路径）。新增
  `match_language_index` 单测 + 清单 `language` 往返/旧 JSON 兼容断言，
  core 120 项全过；双平台 check 0 error。

- 0.1.0（步骤 26–27，#14）：**卸载器多语言 + 环境变量/PATH 管理**（NSIS
  卸载语言与环境惯例对齐批次）：
  ① **卸载器多语言**（NSIS `MUI_UNPAGE_LANGUAGE` 对齐）：卸载清单新增
  `languages`（安装器把 `ui.languages` 随清单内嵌 MIEDUNIN 尾部/落盘
  uninstall.json）；清单携带 >1 语言时卸载器先显示语言选择页（预选系统
  语言，确认后同步窗口标题）；卸载器全部 UI 与日志文案键化（45 个
  `un_*` 键，英文默认表 + 内置 zh-CN 包双语同步，键集一致性测试守护）；
  静默模式按系统语言自动预选。旧清单无该字段 → serde default 空 →
  回退内置 zh-CN（历史中文文案行为不变）。**顺带修复**：语言页/确认页
  取消（Esc/按钮）不再误删清单与自删除卸载器（旧版取消后安装再也无法
  卸载）；卸载器静默日志补 TMPDIR 回退（与安装器 #21 对齐）。
  ② **环境变量/PATH 管理**（NSIS WriteEnvStr 惯例 / Inno
  `ChangesEnvironment` 对齐）：`[[install.env]]`（name/value/part/scope）
  写 HKCU\Environment 或 HKLM\SYSTEM\...\Environment；`all` 直接设值 /
  `path_append`/`path_prepend` 按 `;` 列表 ASCII 不区分大小写去重合并
  （不覆盖已有内容）；写入后广播 `WM_SETTINGCHANGE` "Environment"
  （新启动进程立即可见，无需注销）；卸载精确清理——PATH 式合并只移除
  本安装写入的段（不误伤其他软件的段），`all` 写入还原安装前原值或删除
  新建值；编译器 5.2.3 校验；builder 新增环境变量编辑卡片；`Path` 与含
  `%VAR%` 值写 REG_EXPAND_SZ。纯函数下沉 `miedit_core::envspec`（split/
  merge/remove/expand + 6 单测），决策与清理引用 `EnvCleanupRef` 随清单
  内嵌。新增 8 项单测（文案键集/解析链/清单新旧兼容/EnvSpec 默认值），
  core 119 项全过；双平台 check 0 error；Linux 实机端到端（构建 + 静默
  安装退出码 0 + 清单内嵌语言包 + 独立卸载器读取卸载成功 + payload
  逐字节一致）。

- 0.1.0（步骤 25，#13 — 版本号自此固定）：**压缩格式扩充**（NSIS
  `SetCompressor` 对齐 + 扩展，全部纯 Rust）：`install.compression`
  从 2 档扩到 **6 档**——`deflate`（条目级）/ `solid`（整包 Deflate，
  NSIS solid 风格，原有）+ **`lzma` / `lzma-solid`**（NSIS 旗舰 LZMA：
  `lzma-rust 0.1.7`，tukaani xz-for-java 纯 Rust 移植，sevenz-rust
  同源自带 encoder，preset 6/8MB 字典；此前评估的 lzma-rs 0.3 编码器
  为存根不可用）+ **`brotli` / `brotli-solid`**（brotli crate 纯 Rust，
  quality 9）。归档条目标志位扩展 bit2=brotli、bit3=lzma（bit0/bit1
  不变，旧包兼容，未知标志报错并提示需新版安装器）；压缩/解压纯函数
  下沉 `miedit_core::archive`（`Algo` 枚举 + `compress_bytes`/
  `decompress_bytes` + `normalize_compression` 别名归一化，编译器与
  安装器共用，两 crate 移除 flate2 直接依赖）；安装器 solid 解压按
  配置分派三算法；builder 压缩下拉 6 选项 + 预览同步。新增 6 项单测
  （三算法条目级/整包往返、压缩比对照锁定价值、别名归一化、旧标志
  兼容锚点），core 108 项全过；双平台 check 0 error；Linux 实机 6 档
  端到端（构建 + 静默安装退出码 0 + payload 逐字节一致 + install.log），
  体积排序 lzma-solid < lzma < brotli-solid < brotli < deflate ≈ solid。
  **版本号固定 0.1.0**（用户指令：自本批次起不再递增，协议第 6 条
  已修订；`miedit_core::VERSION` 随 CARGO_PKG_VERSION 自动生效）。

- v0.13.0（步骤 22–24）：**注册表全类型与精确卸载清理 + ExecWait 自定义
  命令 + 组件分组树**（NSIS 成熟度对齐批次）：
  ① `[[registry]]` 值类型从仅 REG_SZ 扩展到**全类型**（NSIS
  `WriteRegDWORD`/`WriteRegBin`/`WriteRegExpandStr`/`WriteRegMultiSz`）：
  `dwords`（十进制/`0x` 十六进制/负数 32 位回绕）、`binary`（十六进制串），
  `expand_sz`、`multi_sz`，加 `view = "default/64/32"`（NSIS `SetRegView`，
  KEY_WOW64_32KEY/64KEY）；winreg 0.52 对 REG_BINARY/REG_EXPAND_SZ 无
  `ToRegValue` 映射，用 `set_raw_value` 手工构造（REG_BINARY 原始字节、
  REG_EXPAND_SZ UTF-16LE+NUL）。② **卸载精确清理**：安装时记录键是否为
  本次新建（`registry_key_exists`），卸载时新建键整树删除、**既有键只删
  本次写入的值并尝试清理空壳键**——不误伤安装前已存在的内容（旧清单无
  `registry_values` 字段回退整树删除，行为不变）。③ **自定义命令**（NSIS
  `Exec`/`ExecWait` / Inno `[Run]` 前后段）：`[[install.exec_before]]`
  （解包前）、`[[install.exec_after]]`（清单/卸载器就绪后）、
  `[[install.exec_before_uninstall]]`（随清单内嵌/落盘，卸载删除阶段前，
  失败不中止卸载）；每条 `cmd/args/working_dir`（`{install_dir}` 模板）+
  `abort_on_error` + `optional`；GUI 与静默同生效；Windows CREATE_NO_WINDOW
  不闪黑框；退出码/输出尾部入日志。④ **组件分组树**（NSIS
  `SectionGroup`/`SectionGroupEnd`）：`[[components]]` 新增
  `group = "父组/子组"`，组件页树形渲染（组行 ▸/▾ 折叠展开、组复选框
  三态联动子组件且必装保持勾选、声明顺序保持、中间组隐式创建），
  安装类型与 CLI `--components=` 仍按扁平索引运作。纯函数下沉
  `regspec`（6 单测）/`runcmd`（4 单测）/`componenttree`（5 单测），
  core 102 项全过；编译器新增 5.2.1 命令校验与 5.2.2 注册表校验；
  builder 组件卡片加 group 输入框；demo 脚本追加三特性示例。

- v0.12.0（步骤 21）：**安装导航 + UI 材质/模板 + Linux/CLI 适配 +
  组件空间联动**（七项，用户指令驱动）：① 盘符切换空间即时刷新回归
  强化（`D:/Demo`→`E:/Demo` 正/反斜杠链路单测）；② 安装导航对齐
  HM NIS Edit——左列步骤导航栏（当前步 accent 高亮/已完成勾点/未到
  空心点，仅指示不可点击），`ui.show_step_rail`（默认 true）；③
  **Linux 适配**——路径模板 Linux 实现（`{autopf}`→`/opt`、XDG 数据
  目录、Windows 风格反斜杠脚本跨平台可用）、`.desktop` 快捷方式
  （applications/桌面目录）、`is_admin`=euid==0、CJK 字体回退链、
  finalize 落盘 `uninstall.json`（卸载器清单文件回退读取）、per_machine
  需 root、CI 新增 ubuntu job（`MIEdit-linux.zip`）；④ **CLI 适配**——
  `--help`/`--version`/`--components=`（Inno /COMPONENTS 语义，GUI 与
  静默同生效）`--no-shortcuts`，退出码 0/1/2 规范；⑤ **液态玻璃/
  亚克力**——`ui.material = "glass"`（默认）半透明卡片 + 高光描边 +
  微透明主按钮，`ui.acrylic`（Win11 22H2+ DWM SystemBackdrop，失败
  自动回退绘制玻璃）；⑥ **五套安装模板**——`ui.template` = aurora /
  midnight / sunrise / forest / mono（整体接管调色板）；⑦ 组件页
  Space required 随勾选联动（NSIS components 语义）。纯函数下沉
  `templates`/`cliargs`/`selection_to_rules`（11 项新单测），core 87 项
  全过；文案新增 6 个 nav_* 键双语同步。

- v0.11.0（步骤 20）：**目录页空间动态刷新 + 安装目录校验**（NSIS
  DirectoryPage / `AllowRootDirInstall` 对齐，修复用户报告缺陷）：
  切换盘符后剩余空间不刷新——`disk_free()` 此前基于仅在开始安装时才
  更新的 `dest`，现改为基于目录页**实时输入目录**（`current_dest()`
  跟随 `dir_text`，去空白/包裹引号/裸盘符 `C:`→`C:\` 规范化），空间
  显示、Enter 校验、按钮禁用三处口径统一；目录校验：空输入/相对路径/
  非法字符（`<>|?*"` 与盘符外冒号）/**不存在的盘符**拒绝前进并显示
  `dir_invalid` 提示，盘根/UNC 共享根默认拒绝（`dir_root_warning`），
  `install.allow_root_dir_install = true` 放行（NSIS
  `AllowRootDirInstall` 安全默认语义）；`disk_free_space` 无已存在
  祖先目录时返回 None（不再误回退 C 盘）；决策纯函数下沉
  `miedit_core::installdir`（9 项单测），core 74 项测试全过。

- v0.10.0（步骤 18–19）：**完成详情停留页 + 卸载器壁纸**（NSIS
  `MUI_PAGE_INSTFILES` 完成态与 Inno 卸载背景对齐）：
  安装/卸载完成后不再直接跳完成页，而是停留在**详情页**（显示完成
  状态 + 全量日志），用户点「下一步」进入完成页——安装器 `install_finished`
  状态（Install 页渲染完成横幅与「下一步」，Enter 放宽），卸载器新增
  `Phase::Summary`（完成徽标 + 统计 + 日志默认展开）；卸载器壁纸：
  `install.uninstall_background` → 包尾新分区（Trailer **v4 MIEDPKG4**，
  v3 超集 + 壁纸分区，132B，读取兼容 v2/v3）→ finalize 内嵌进卸载器
  （MIEDUNIN 32B footer +bg_len，兼容旧 24B）→ 卸载器全窗贴图 +
  半透明遮罩（窗口放大 800×540），无壁纸走内置渐变；7 项新单测
  （v4 编解码/壁纸分区往返/旧 v3 兼容/卸载数据新旧格式往返），
  core 65 项测试全过。

- v0.9.0（步骤 17）：**ARP ModifyPath 修复入口 + 修复模式**（Inno `ModifyPath`
  / 修复向导对齐）：卸载注册表键不再写 `NoModify=1`，改写 **`ModifyPath` =
  `"<安装器路径>" --repair`**（控制面板「更改/修复」按钮启动安装器修复；
  `NoRepair=1` 保留，路径异常时保守回退指向卸载器）；安装器解析
  `--repair` / `/repair` 开关，跳过全部向导页直达**修复确认页**（标题/
  正文/安装目录 + 修复摘要统计：文件规则/快捷方式/注册表值个数），
  确认后全量恢复组件选择（`repair_selection` 全选，install_type 归
  Custom）重走常规安装流程（重解压全部文件 + 重建快捷方式/注册表），
  完成页切换为「修复完成」文案；静默修复（`--silent --repair`）等同
  静默安装语义；文案新增 6 键（repair_title/body/summary/now/
  finish_title/finish_body，内置 zh-CN + 英文默认双包同步）；核心逻辑
  下沉 `miedit_core::repair`（modify_path_value 引号拼装、repair_selection、
  summarize 统计口径与 finalize 实际行为对齐），5 项单测。向后兼容：
  旧包无新字段、修复入口由运行期生成，旧安装包重新安装后即获得修复入口。

- v0.8.0（步骤 15）：**多语言框架**（NSIS MUI2 `MUI_LANGUAGE` 对齐）：
  `[[ui.languages]]`（id + 语言选择页显示名 + 文案覆盖键值表），内置
  **zh-CN（完整中文 56 键）/ en-US（英文默认）** 双语包，按 id/别名
  （`zh`/`zh-Hans`→zh-CN）匹配补全缺键；文案解析链下沉
  `miedit_core::config::resolved_text`（`ui.text` 单键覆盖 > 语言包
  `text` > 内置包 > 英文默认，`ui_text` 旧入口等价 `resolved_text(None)`，
  旧包行为不变）；定义 >1 个语言时欢迎页前显示**语言选择页**（NSIS
  语言选择语义，单向不可回退），启动按系统 UI 语言自动预选
  (`ui.language_auto_detect`，默认开启：Windows `GetUserDefaultUILanguage`
  LANGID→BCP 47 映射表，非 Windows 解析 `LC_ALL`/`LANG`；精确匹配优先，
  其次主标签匹配，否则第一个），单语言直接使用；语言切换后窗口标题同步。
  原 UI 硬编码中文文案（版本行/许可滚动提示/目录非空警告/进度统计/
  细节开关等）全部文案化，英文默认表修正为全英文基调（原中文键值移入
  内置 zh-CN 包）；安装后台线程（finalize 日志）与静默模式按选中语言渲染。
  编译器校验语言包 id 格式/空 name/重复 id；8 项单测（解析链优先级、
  内置包键集一致性、LANGID 映射、环境变量解析、语言匹配、启动预选）。

- v0.7.0（步骤 12–14）：**最小系统版本门槛 + 快捷方式增强 + 卸载完成页统计**
  （Inno `MinVersion` / `[Icons]`、NSIS 卸载统计对齐）：
  ① `install.min_version`（如 "6.1"/"10.0.19045"）安装器启动时用 ntdll
  `RtlGetVersion` 取真实版本比对（手工 extern 声明，避开 GetVersionEx 兼容性
  垫片谎报与 windows-rs 0.58 未导出问题），低于门槛 GUI 弹错退出、静默写日志
  返回 1；解析/比较纯函数下沉 `miedit_core::sysreq`（3 项单测），编译器拒绝
  无效门槛。
  ② `[[shortcuts.entries]]`（Inno [Icons]）：自定义名称/目标/参数/子目录/图标/
  工作目录/描述，位置多选（start_menu / desktop / quick_launch，新增
  `winutil::quick_launch_dir`），与 start_menu/desktop 简单开关叠加兼容；
  目录去重记录进卸载清单，卸载时快捷方式目录按深度先删（支持嵌套子目录）；
  单条目失败仅告警不阻断安装；编译器校验 name/locations。
  ③ 卸载完成页统计（NSIS 卸载统计）：RunState 新增已删除文件数/目录数/
  释放字节数（删除前取 metadata；用户数据目录删除前递归统计
  `util::dir_file_stats`，含真实临时目录单测），完成页与静默日志展示；
  卸载器还修复了原快捷方式目录删除顺序问题（父目录先删导致失败报错）。
  旧包/旧脚本无新字段时行为不变（serde default）。

- v0.6.0（步骤 10–11）：**InstallTypes 安装类型 + 组件页描述侧栏**（NSIS
  `InstallTypes` / `components.c` 描述框对齐）：`[[install_types]]`
  （name + 默认勾选的组件索引 + default 标记）在组件页顶部渲染类型单选行，
  末尾自动附带 **Custom**；选中类型 → 组件勾选重置为该类型定义（必装强制勾选、
  越界索引宽容忽略），手动改动任意组件不再命中任何类型时自动跳到 Custom
  （NSIS 语义）；「重置」按钮在有类型时回到默认类型；启动时按默认类型初始化
  勾选（旧包无类型时保持全选兼容）。类型状态机下沉
  `miedit_core::installtypes` 纯函数（`selection_for_type` / `matching_type` /
  `default_type_index`，含往返互逆不变量等 7 项单测），编译器校验类型引用
  （索引越界 / 空 name / 多个 default 即报错），构建器 GUI 新增安装类型编辑区。
  组件描述改为 NSIS 式描述框：悬停/点击组件行 → 下方描述面板跟随显示
  `description`（必装组件带「必需」徽标，空描述显示占位文案），替代原逐条内嵌描述。
  另新增 `.github/workflows/release.yml`：手动触发（`workflow_dispatch`）
  编译 workspace 并上传单层 zip（产物直接位于 zip 根目录）。

- v0.5.0（步骤 8–9）：**覆盖失败重试交互 + 安装期重启替换**（NSIS 文件错误对话框 /
  `SetRebootFlag` 对齐）：安装写文件失败时按覆盖策略分流——`overwrite = try`
  直接跳过（同 NSIS 语义）；其余策略在 GUI 中弹出「重试 / 忽略 / 中止」模态卡片
  （egui 遮罩 + 卡片，安装暂停等待用户选择），重试循环、忽略记录日志并继续、
  中止提示已写入文件去向（NSIS 中止不清除已写入文件）；静默模式无 GUI，
  默认忽略并计划重启替换（无人值守升级尽量完成，留下可追溯日志）。
  重启替换由 `miedit_core::winutil::schedule_replace_on_reboot` 承担：优先把被占用
  旧文件**改名腾位**（`.miedel`）→ 立即写入新内容 → 旧文件调度重启删除
  （`PendingFileRenameOperations`）；改名失败则旁路 `.mienew` +
  `MoveFileExW(DELAY_UNTIL_REBOOT | REPLACE_EXISTING)` 计划重启时原子替换。
  待替换文件记入卸载清单（保证可清理）并展示在完成页琥珀提示中
  （"需要重启以完成安装"）。决策纯函数与「请求-应答」信箱下沉
  `miedit_core::fileerror`（PromptChannel 协议不变量、处置映射等 10 项单测）。
- v0.4.0（步骤 7）：**包完整性 CRC32 校验 + `--ncrc`**（NSIS `CRCCheck` 对齐）：
  包尾升级为 `MIEDPKG3`（116B：v2 范围表 + payload/config/uninstaller 三分区
  CRC32）；安装器启动时流式校验，失败报"安装包已损坏"并提示分区与期望/实际值；
  `--ncrc` / `/ncrc` 跳过（CRCCheck off/force 语义）；旧格式包（MIEDPKG2）
  仍可读取安装（无校验直接放行）。CRC32 为 `miedit_core::crc32` 纯 Rust
  查表实现（无新依赖，标准校验值单元测试），校验逻辑下沉 `trailer::verify_crc`
  （含真实临时包文件的篡改回读测试：篡改一字节必报错）。
- v0.3.0（步骤 6）：安装器与卸载器 EXE 嵌入 **VERSIONINFO 版本信息资源**
  （NSIS `VIProductVersion` / `VIAddVersionKey` 对齐）：
  ProductName / ProductVersion / FileVersion / CompanyName / FileDescription /
  LegalCopyright 取自 `[app]`（name/version/publisher，空值跳过）；
  VS_VERSIONINFO 字节序列由 `miedit_core::versioninfo` 纯函数构造（含结构回读单元测试）；
  图标与版本信息在同一 `UpdateResourceW` 事务写入（卸载器为 EXE 内嵌前预写 RT_VERSION）。
  Windows 资源管理器属性可见性待实机验证。
- v0.2.0（步骤 1–5）：SetOverwrite 覆盖策略（含 ifnewer + 源时间戳保留）、
  卸载 Delete /REBOOTOK 重启删除、卸载前运行检测、QuietUninstallString、install.log 落盘。
