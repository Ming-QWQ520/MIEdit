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
| 10 | InstallTypes 安装类型（Typical/Full/Minimal/Custom） | NSIS `InstallTypes` / Inno `[Types]` | 待办 |
| 11 | 组件页描述侧栏（组件联动描述面板） | NSIS `components.c` 描述框 | 待办 |
| 12 | 最小系统版本门槛 | Inno `MinVersion`/`OnlyBelowVersion` | 待办 |
| 13 | 快捷方式增强：快速启动栏、独立 lnk 任务 | Inno `[Icons]` | 待办 |
| 14 | 卸载完成页统计（文件数/回收空间） | NSIS 卸载统计 | 待办 |
| 15 | 多语言框架（MUI 式语言选择 + 文案包） | NSIS MUI2 | 待办 |
| 16 | zstd 压缩档 | Inno `Compression.Zstd.pas`（等纯 Rust 编码器成熟） | 待办 |
| 17 | ARP ModifyPath/修复入口评估 | NSIS/Inno 卸载注册表扩展 | 评估中 |

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

### 步骤 17：ARP ModifyPath/修复入口
- **做什么**：评估在卸载注册表键写 `ModifyPath`（启动安装器向导到修复模式）的可行性；
  涉及安装器"修复模式"（重解压 + 重建快捷方式/注册表），规模较大，先出设计再实现。

## 工程约束（每次迭代都要遵守）

1. **纯 Rust**：不引入 C 构建链；新增依赖须在 Windows/Linux 双平台可编译。
2. **向后兼容**：旧格式安装包/卸载清单必须仍可处理（serde default / magic 回退）。
3. **测试**：核心逻辑写成 `miedit-core` 纯函数并加单元测试；`cargo test -p miedit-core`
   在无 Windows 环境下也必须通过；UI 交互逻辑以 `cargo check --workspace` 保证可编译。
4. **小步提交**：一次迭代一个步骤，提交信息格式
   `feat: NSIS 对齐 #N — <内容>`（作者：Ming-QWQ520 <192413597+Ming-QWQ520@users.noreply.github.com>）。
5. **文档同步**：完成步骤后更新本文件状态表、README 特性表与 lessons 文档。
6. **版本号**：功能对齐批次完成后递增 `Cargo.toml` workspace version。

## 变更日志

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
