# NSIS / Inno Setup 源码研究：取长补短到 MIEdit

> 参考源码已克隆至本仓库（`git clone --depth 1`）：
> - `references/nsis` — NSIS-Dev/nsis（C/C++，makensis 编译器 + exehead 安装器运行时）
> - `references/issrc` — jrsoftware/issrc（Delphi/Pascal，编译器 + IDE + WizardForm）
>
> 本文记录我们研究了哪些实现、MIEdit 借鉴了什么、以及有意不采纳的部分。

## 一、两个项目的架构与优缺点速览

### NSIS（`references/nsis`）

| 方面 | 实现 | 值得借鉴 |
| --- | --- | --- |
| 脚本引擎 | `Source/script.cpp` / `tokens.cpp`：指令式脚本，`CEXEBuild` 逐行编译为压缩指令流 | 脚本可编程性强；MIEdit 保持声明式 TOML，但吸收 **组件/节（Section）模型** |
| 压缩 | `Source/build.cpp::CEXEBuild::set_compressor(compressor, solid)`（第 4089 行）：支持 zlib / lzma / **lzma-solid**，solid 时切换到 `_solid` 专用 stub | ✅ **整包 LZMA solid** —— 大幅提升压缩率，已实现（`install.compression = "lzma-solid"`） |
| 安装器运行时 | `Source/exehead/fileform.h`（自定义数据块格式 + firstheader/crc）、`ui.c` / `components.c`（向导页） | 数据块**自描述头部 + CRC** 思路；MIEdit 用包尾 Trailer + 校验范围，效果等价 |
| 目录记忆 | `InstallDirRegKey` 指令（`tokens.cpp`/`script.cpp`）：升级安装时从注册表恢复上次目录 | ✅ 已实现 `winutil::read_prev_install_dir`（按 app_id 读 Uninstall 键） |
| 卸载器 | 生成 `un_*.exe`：先复制自身到临时目录再运行，避免自删除死锁 | ✅ MIEdit 卸载器同样“重命名自身 + 分离 cmd 延迟删除”，已实现 |
| 不足 | C 语言运行时扩展困难；脚本语法学习曲线陡峭；UI 非常老旧 | MIEdit 用 Rust + egui，UI 现代化、类型安全 |

### Inno Setup（`references/issrc`）

| 方面 | 实现 | 值得借鉴 |
| --- | --- | --- |
| 向导页架构 | `Projects/Src/IDE.Wizard.WizardForm.pas`：`TNewNotebook` + 每步一个 Page（WelcomePage/AppInfoPage/AppDirPage/AppFilesPage/AppIconsPage/...） | ✅ MIEdit 构建器改为 **同款分步向导**（10 步），页脚 ⟨返回/下一步/取消⟩ |
| 压缩 | `Projects/Src/Compression.*.pas`：LZMA1 小型解压器、LZMA2、**Zstd（6.3+）**、zip | 压缩算法可插拔思想；MIEdit 目前 deflate/lzma-solid，后续可加 zstd |
| 压缩调度 | `Compiler.CompressionHandler.pas`：solid 与分文件压缩统一调度 | 参考 MIEdit `files_map` 按组件过滤时的进度换算 |
| 就绪页 | `wpReady`（安装前配置摘要） | ✅ 构建向导第 10 步顶部加入 **配置摘要块** |
| DirExistsWarning | 目录已存在非空时提示 | ✅ 安装器目录页已加琥珀色提示 |
| 卸载数据 | `Uninstallable=yes` + `CreateUninstallRegKey`，` unins*.dat` 日志 | ✅ MIEdit `user_data_dirs` 可选删除（语义更清晰） |
| 不足 | Pascal Script 引擎庞大；IDE 向导仍为 Win32 控件风格 | MIEdit 的向导是自绘 UI，视觉现代、可完全定制 |

## 二、本次落地的借鉴清单

1. **solid 整包压缩**（NSIS `set_compressor(solid)`）：`install.compression = "solid"`；
   编译器条目原样存储、整包 Deflate 压缩（592KB → 103KB），安装器整包解压后走原归档解析。
   原计划用 LZMA solid，但发现纯 Rust 的 `lzma-rs 0.3.0` XZ **编码器是存根**
   （`src/encode/lzma2.rs::encode_stream` 直接按 LZMA2 未压缩块写出，见下方“重要发现”），
   故先落地 Deflate solid，待成熟 LZMA 编码器（如 sevenz-rust 系）后切换。
2. **InstallDirRegKey**（NSIS）：安装器启动时按 `app_id` 从 HKLM/HKCU 卸载键读 `InstallLocation`，
   升级安装自动定位上次目录。
3. **wpReady 摘要**（Inno）：构建向导第 10 步顶部显示配置摘要（应用/目录/文件/组件/压缩方式）。
4. **DirExistsWarning**（Inno）：目录页目标已存在非空时提示“将被覆盖或合并”。
5. **向导页架构**（Inno WizardForm + HM NIS Edit 脚本向导）：构建器重构为 10 步分步向导；
   **第 9 步为“项目脚本”页** —— 类似 HM NIS Edit 生成脚本后可编辑：查看/编辑/应用修改/重新生成/另存为。

## 三、研究了但有意不采纳（及原因）

- **NSIS 指令式脚本 VM**：解释器+栈机的灵活性远超声明式配置，但复杂度/安全面也大；
  MIEdit 坚持声明式 TOML + 向导，用“脚本页”满足高级用户直接编辑的需求。
- **Inno Pascal Script**：运行时脚本引擎（CodeClasses.pas 等）体量巨大，与纯 Rust 目标冲突。
- **Inno 多语言 [Languages]**：MIEdit 用 `ui.text` 覆盖表实现单语言定制，多语言运行时切换待后续。
- **插件体系（NSIS Plugins API）**：需要 C ABI 插件约定，当前阶段不引入。
- **7-ZIP / external DLL 压缩器**（issrc `Compression.SevenZipDLLDecoder.pas`）：
  依赖外部 DLL，违背“单文件、纯 Rust”目标；MIEdit 用纯 Rust flate2/lzma-rs（解码）实现。

## 四、后续可继续吸收的源码线索

- NSIS `Source/exehead/fileform.h`：firstheader 的长度/时间戳/CRC 字段 —— 可为 MIEdit Trailer 增加 CRC32。
- NSIS `Source/exehead/ui.c`：组件页“组件描述”面板联动 —— MIEdit 组件页可加描述侧栏。
- Inno `Compression.Zstd.pas`：Zstd 拉起/卸载函数表 —— MIEdit 可加 zstd 条目压缩档位。
- Inno `Compiler.SetupCompiler.pas`：`MinVersion`/`OnlyBelowVersion` 检查 —— 安装器可加系统版本门槛。

## 五、落地清单 · 第二批（NSIS 安装/卸载行为对齐 #1）

以 NSIS 为主的安装/卸载**运行时行为**对齐（v0.2.0）：

1. **`SetOverwrite` 覆盖策略**（NSIS：脚本指令编译进指令流，exehead 写文件路径按策略决策）：
   `install.overwrite = on | off | try | ifnewer`。决策逻辑独立为 `miedit_core::overwrite`
   （纯函数 + 单元测试）；编译器校验非法值并在 `files_map.FileRef` 写入源文件修改时间；
   安装器写盘成功后用 `SetFileTime` 回设目标时间戳（NSIS 保留源时间戳的做法），
   保证下一次 `ifnewer` 升级比较语义正确。
2. **`Delete /REBOOTOK` 重启删除**（NSIS：被占用文件不视为失败，`MoveFileEx(MOVEFILE_DELAY_UNTIL_REBOOT)`
   写入 `PendingFileRenameOperations`）：卸载器删除失败时自动转入重启删除，
   `RunState.reboot_scheduled` 计数，完成页与静默日志分别提示。
3. **卸载前运行检测**（NSIS `un.onInit` 检测应用窗口 / Inno Restart Manager）：卸载清单新增
   `main_exe`（serde default 向后兼容旧清单），确认页显示琥珀色警告 + “重新检测”按钮，
   静默模式记录警告后继续。
4. **`QuietUninstallString`**（NSIS / ARP 自动化惯例）：卸载注册表键补充
   `"uninstall.exe" /S` 值。
5. **`install.log` 落盘**（NSIS `LogSet on` 语义）：安装完成把累计日志写入
   `$INSTDIR\install.log`，`install.write_log = false` 可关闭。

有意后续再做：覆盖失败时的 Retry/Ignore/Abort 交互（NSIS 细粒度文件对话框）、
`/NCRC` 与包 CRC32（见 `nsis-alignment-roadmap.md` 步骤 3）。

## 六、落地清单 · 第三批（NSIS 对齐 #2，v0.3.0）

1. **EXE 版本信息资源**（NSIS：`VIProductVersion` + `VIAddVersionKey` 由 makensis
   写入安装器与卸载器 EXE 的 RT_VERSION 资源，资源管理器属性可见）：
   MIEdit 编译器在构建时给**安装器与卸载器 EXE 都嵌入 VERSIONINFO**——
   ProductName / ProductVersion / FileVersion 取 `[app].name/version`，
   CompanyName / LegalCopyright 取 `[app].publisher`（空值跳过对应键），
   FileDescription 为"{应用名} 安装程序 / 卸载程序"。
   VS_VERSIONINFO 二进制结构（头部 6 字节 + UTF-16LE + 32 位对齐 +
   VS_FIXEDFILEINFO + StringFileInfo/VarFileInfo）由 `miedit_core::versioninfo`
   **纯函数手工构造**，配结构回读单元测试；PE 写入复用 icon.rs 的
   `UpdateResourceW` 链路并与图标替换**合并为同一事务**（`patch_pe_resources`，
   先图标后版本，一次 Begin/End）。卸载器在编译期写临时文件预写 RT_VERSION
   后再内嵌进安装包（其清单/图标仍按原设计安装期追加尾部，互不影响）。
   版本号宽容解析：`1.2.3` → dwMS=1.2/dwLS=3.0（NSIS 严格要求 x.x.x.x，
   这里对自由格式脚本取数字前缀，超出 65535 截断）。

## 七、落地清单 · 第四批（NSIS 对齐 #3，v0.4.0）

1. **包完整性 CRC32 校验**（NSIS：`Source/exehead/fileform.h` firstheader 自带
   length/ timestamp/CRC 字段，exehead 启动即校验，`CRCCheck on/off/force`
   三态 + `/NCRC` 开关）：MIEdit 包尾升级为 **`MIEDPKG3`**（116B = v2 范围表
   + payload/config/uninstaller 三个分区的 CRC32）；安装器 `load_self_package`
   启动时**流式校验**（64KB 分块，不整包载入内存），失败报"安装包已损坏"
   （含分区名与期望/实际校验值）；`--ncrc` / `/ncrc` 跳过。
   兼容策略：CRC32 为 `miedit_core::crc32` 纯 Rust 查表实现（零新增依赖，
   `"123456789" → 0xCBF43926` 标准校验值测试）；旧格式包（MIEDPKG2）读取时
   自动回退、无校验直接放行——新代码兼容旧包，旧安装器不读新包（与 NSIS
   firstheader 版本字段语义一致）。校验逻辑下沉 `trailer::verify_crc`，
   测试用真实临时包文件做"篡改一字节必报错"的三分区回读验证。

## 八、落地清单 · 第五批（NSIS 对齐 #4，v0.5.0）

1. **覆盖失败重试交互**（NSIS exehead 文件错误对话框：写入失败弹
   重试/忽略/中止，模态、安装暂停）：MIEdit 用 egui 遮罩 + 模态卡片实现，
   后台安装线程与 UI 线程之间以 `miedit_core::fileerror::PromptChannel`
   「请求-应答」信箱通信（单测验证协议不变量：答复必须在有请求时才生效、
   一次请求只消费一次、新请求清掉残留答复）。按覆盖策略分流：
   `overwrite = try` 失败不弹窗直接跳过（NSIS SetOverwrite try 语义）；
   `on / off / ifnewer` 失败才征求用户意见。中止不清除已写入文件（同 NSIS），
   错误信息说明已写入 N 个文件保留、可稍后用卸载器清理。
2. **静默模式的默认处置（设计取舍）**：静默安装无 GUI、无法弹窗征求意见，
   取「忽略 + 计划重启替换」为默认——无人值守升级尽量完成，且每个决策都
   落入 install.log / %TEMP% 日志可追溯。这是 MIEdit 自己的选择（并非声称
   NSIS 静默模式同款行为），换来的是自动化场景下被占用文件不会中断升级。
3. **安装期重启替换**（NSIS `SetRebootFlag` / Inno `restartreplace`）：
   `winutil::schedule_replace_on_reboot` 双路径——优先把被占用旧文件**改名腾位**
   （同卷 rename 对被占用文件通常可行，与 NSIS 处理运行中 EXE 的思路一致），
   新内容立即写入，旧文件登记 `PendingFileRenameOperations` 重启删除；
   改名失败（目录锁/权限）时回退旁路 `.mienew` +
   `MoveFileExW(DELAY_UNTIL_REBOOT | REPLACE_EXISTING)` 重启时原子替换。
   待替换文件照常记入卸载清单（卸载器后续可清理），完成页琥珀提示
   "需要重启以完成安装"并列出文件。旧文件腾位名固定 `.miedel` 后缀、
   旁路名 `.mienew`，先清残留再写入，避免多次升级堆积垃圾。
4. **egui 0.29 模态实现细节**：`Area` 已无 `fixed_size`，全屏遮罩用
   `anchor(LEFT_TOP) + allocate_exact_size(screen_rect().size(), Sense::click())`
   画半透明黑并吞掉模态外点击；模态卡片置于 `Order::Foreground`。

> 对齐路线图见 [nsis-alignment-roadmap.md](nsis-alignment-roadmap.md)，按步骤持续迭代。

## 九、落地清单 · 第六批（NSIS 对齐 #5，v0.6.0）

1. **InstallTypes 状态机**（NSIS `InstallTypes` / `SectionSetFlags`）：
   MIEdit 用「类型 → 勾选快照」+「勾选 → 类型反查」两个纯函数构成互逆状态机
   （`miedit_core::installtypes`，单测验证往返自洽）。NSIS 的 Custom 是下拉里
   最后一个固定项；MIEdit 同样把 Custom 作为 `types.len()` 伪索引渲染在单选行
   末尾，手动改动任意组件 → `matching_type` 不再命中 → 自动落回 Custom。
   必装组件的勾选在两个方向上都被强制归一化（NSIS 必选 Section 语义），
   因此「Full + 取消必装」不会污染类型匹配。类型定义里越界的组件索引宽容忽略
   （旧脚本/手写脚本容错），编译器则直接报错（两层防线）。
2. **`default = true` 的语义**：NSIS 默认选第一个 InstType；
   MIEdit 允许显式标记默认类型（编译器拒绝多个 default），未标记时同样回退
   第一个。启动时组件页直接按默认类型初始化勾选，而不是全选——与 NSIS
   进入组件页时的状态一致；旧包（无 `[[install_types]]`）保持全选兼容。
3. **组件描述框**（NSIS `components.c` 描述框 / Inno TypesDescription）：
   NSIS 的描述跟随鼠标悬停（WM_MOUSEMOVE 轮询 section 矩形），Inno 则是
   点击选中更新。MIEdit 两者都做：悬停或点击组件行 → 下方描述面板跟随。
   必装组件的复选框处于禁用态，egui 禁用控件的 Response 不含 hovered，
   因此用整行 `ui.interact(rect, id, Sense::hover())` 作为悬停面兜底。
   原逐条内嵌描述文本移除，避免与描述面板重复；空描述显示占位文案。
4. **CI 形态取舍**：产物面向 Windows（PE 资源替换 / UAC / lnk），
   CI 跑 `windows-latest` + `workflow_dispatch` 手动触发即可，不需要 PR/推送
   自动构建；zip 用 `Compress-Archive` 打平（`$stage\*` 而非 `$stage`），
   保证解包一层直接见到 exe，不做 Release 发布（版本化发布留给后续里程碑）。

## 十、落地清单 · 第七批（NSIS 对齐 #6，v0.7.0）

1. **真实 Windows 版本必须走 RtlGetVersion**（Inno MinVersion / `OnlyBelowVersion`
   的前置）：`GetVersionEx` 在没有 manifest 声明的进程里被兼容性垫片谎报为
   6.2（Win8），无清单的 stub EXE 正中此坑。windows-rs 0.58 未导出
   `RtlGetVersion`（SystemInformation 只有类型没有函数），改为手工
   `#[link(name = "ntdll")] extern "system"` + 最小 `OSVERSIONINFOW` 镜像
   （5 个 u32 + 128 个 u16），跨 windows-rs 版本零风险，也免去多开一个 feature。
   门槛解析/比较（`sysreq`）是纯函数下沉 core 并单测，编译期拒绝无效写法，
   运行期对旧包宽容忽略。
2. **快捷方式条目与清单的目录去重**（Inno `[Icons]`）：`[[shortcuts.entries]]`
   与旧 `start_menu/desktop` 开关叠加生效——同一开始菜单分组目录可能被两路
   代码各记一次，卸载时第二次删除必然报错。用 BTreeSet 去重后才写进
   `shortcut_dirs`；卸载器同时改为**深度优先**删快捷方式目录（原实现按记录
   顺序删，嵌套子目录时父目录先删必然失败）。quick_launch 没有公共目录变体
   （Inno `{userquicklaunch}` 同样只有用户级），per_machine 安装写入当前用户
   目录，系统级目录一律不记入清单（只删 lnk 不删容器）。
3. **卸载统计的实现取舍**（NSIS 卸载统计）：字节数在删除**前**取
   `metadata().len()`（删除后就取不到了）；删除失败/重启删除的文件不计入
   "已释放"，避免虚报。用户数据目录用 `dir_file_stats` 递归统计（walkdir，
   真实临时目录单测）。GUI 完成页与静默日志共用同一组 RunState 计数，
   两条路径数字一致。
4. **windows-msvc 交叉检查进入常规流程**：本轮 `RtlGetVersion`、FILETIME 类
   cfg(windows) 错误在 Linux 沙箱完全不可见，`cargo check --workspace
   --target x86_64-pc-windows-msvc`（不链接、无需 MinGW）已两次在提交前拦下
   真实错误，列为每次迭代的固定验证步骤。

## 十一、落地清单 · 第八批（NSIS 对齐 #7，v0.8.0）

1. **多语言的"内置包打底"而非"全量复制"**（NSIS MUI2 `MUI_LANGUAGE` 的
   落地形态）：NSIS 每个 `LangString` 都要逐语言写全，缺一个运行期显示空。
   MIEdit 反过来——内置 zh-CN 包（56 键全量中文）与 en-US 包（英文默认表）
   常驻编译进安装器，脚本 `[[ui.languages]]` 只需写 `id` + 显示名即可获得
   完整文案，`text` 表只写想改的键。id 按精确/别名（`zh`、`zh-Hans` 均命中
   zh-CN）匹配内置包；未知 id（如自定义 `fr-FR`）键值逐级回退英文默认，
   不会出现空文案。键集一致性由单测锁定（zh-CN 包必须覆盖英文默认表全部键）。
2. **`ui.text` 保持最高优先级是向后兼容的关键**：旧脚本靠 `ui.text` 做
   单键覆盖（demo 的整套中文），多语言引入后如果语言包优先级更高，旧包
   行为就会突变。解析链定为 `ui.text` > 语言包 `text` > 内置包 > 英文默认，
   `ui_text(cfg, key)` 旧入口等价 `resolved_text(cfg, None, key)`——无语言包
   的旧安装包逐字节同行为。代价：`ui.text` 的覆盖是语言无关的（对所有
   语言生效），多语言脚本应把差异文案写进各语言包而不是 `ui.text`。
3. **语言选择页的 NSIS 语义对齐**：NSIS 多语言弹 `LangDLL` 对话框、
   单语言静默；MIEdit 用向导首页 `Page::Language`（>1 语言才出现，单向
   不可回退），预选逻辑 = 精确 id 匹配 > 主标签匹配 > 第一个，预选来源
   `GetUserDefaultUILanguage`（LANGID→BCP 47 映射表：精确表 + 主语言回退，
   完全未知返回 None 走第一个）。静默模式 `/S` 无 UI，直接按自动检测结果
   渲染日志。语言切换后 window_title 需要手动发 `ViewportCommand::Title`。
4. **文案化的最后一公里**：内置英文默认表里残留 5 个中文键（file_error_*、
   finish_reboot_required）与 UI 里 7 处硬编码中文（版本行、许可滚动提示、
   目录非空警告、进度统计、细节开关、重启剩余计数）让双语包永远无法自洽，
   本轮全部文案化并修正为全英文基调（中文值移入内置 zh-CN 包）。教训：
   新增 UI 文案必须走文案表，硬编码字符串会同时污染两个语言包的键集。
5. **安装后台线程也要拿到语言**：`install::spawn` 线程里 finalize 的
   "写入卸载程序与快捷方式..." 日志行原走 `ui_text`（无语言上下文），本轮
   把 `language: Option<usize>` 一路传到 `finalize_install`（GUI 从
   `InstallerApp.language`、静默从 `initial_language`），保证日志与 UI
   同语言。语言索引是配置下标，解析仍走 `resolved_text(cfg, lang, key)`，
   不复制文案字符串，避免快照与配置漂移。

## 十二、落地清单 · 第九批（NSIS 对齐 #8，v0.9.0）

**ARP ModifyPath 修复入口 + 修复模式**（步骤 17，Inno `ModifyPath`/修复向导对齐）。

1. **修复入口的核心是注册表三值协同**（ARP 协议）：`UninstallString`（卸载）、
   `QuietUninstallString`（静默卸载）、`ModifyPath`（更改/修复）。此前写死
   `NoModify=1` 等于主动放弃修复入口；对齐 Inno 后改为写
   `ModifyPath = "<安装器路径>" --repair`，`NoRepair=1` 保留（无独立修复按钮
   语义，「更改」即修复入口）。路径异常时保守回退指向卸载器，绝不让
   ModifyPath 指向不存在的文件。
2. **修复模式复用安装流程而非另写一套**：Inno 修复 = 重解压全部文件 +
   重建快捷方式/注册表。MIEdit 的 `Page::RepairConfirm` 确认页只做一件事——
   把组件选择恢复为全选（`repair_selection`），然后走既有 `start_install`
   流程。修复与安装共享同一条代码路径，不存在"修复遗漏了安装期修复项"
   的分叉风险。install_type 归位 Custom（全选态）与组件页状态机自洽。
3. **统计口径要与实际行为一致**（确认页 `summarize` 的教训）：修复摘要
   展示"将重建 N 个快捷方式"——统计口径照抄 `finalize` 的真实创建逻辑：
   开始菜单组固定有卸载 lnk（+1），有主程序才有程序 lnk（+1）、桌面依赖
   主程序存在；自定义条目按 locations 声明数计（目标无效运行期会跳过，
   静态统计允许少量虚高）。首个单测断言就抓出少算 1 个卸载 lnk 的口径错。
4. **GUI 入口测试的盲区仍在启动参数解析**：`--repair` 解析加进 main 后
   忘了把标志传入 `real_main`，Linux `cargo check` 的输出被 tail 截断漏看
   了 E0425，Windows 交叉检查第一遍就抓住。教训重申：`real_main` 参数
   演进时同步改两处（调用点 + 签名）；验证命令必须全量 grep `^error`，
   不能只看尾部几行。
5. **步骤 16（zstd）维持阻塞并记录在案**：纯 Rust 生态 `ruzstd` 仅有解码器，
   无成熟 zstd 编码器；官方 `zstd` crate 为 C 绑定（cc 构建链），违反纯
   Rust 约束。按协议在路线图记录阻塞说明后转向步骤 17，不空转不硬塞
   不合格依赖。


## 十三、落地清单 · 第十批（NSIS 对齐 #9，v0.10.0）

**完成详情停留页 + 卸载器壁纸**（步骤 18–19，用户直接反馈驱动）。

1. **NSIS 的 INSTFILES 完成态是"停留"而非"跳转"**：MUI2 页面流里
   INSTFILES 完成后停在原页（Show Details 可展开日志），按钮变为
   「下一步」进入 FINISH 页。MIEdit 此前完成即跳完成页，用户失去了
   查看安装明细的机会。落地要点：poll_state 不再自动切页，改设
   `install_finished` 标志；详情页渲染完成横幅 + 「下一步」按钮；
   Enter 键在完成态放宽（未完成时保持忽略防误触）。
2. **卸载器同样要有 Summary 停留页**：卸载器 Phase 状态机插入
   `Summary`（Working 完成且无错误 → Summary → 用户点「下一步」→
   Done 总结页）；有错误仍直接进 Failed（NSIS 失败时 abort 语义，
   详情已在错误页展示）。统计行在 Summary 与 Done 两页各出现一次
   是有意的——详情页给"刚刚发生什么"，总结页给"最终结果"。
3. **格式扩展的版本阶梯**：包尾 Trailer 升 v4（`MIEDPKG4` = v3 超集
   + 卸载器壁纸分区，132B）；卸载器尾部 MIEDUNIN footer 从 24B 升
   32B（+bg_len）。两条格式都遵循同一纪律：encode 写最新版，decode
   逐级回退（v4→v3→v2 / 32B→24B footer→uninstall.json），旧文件永远
   可读。判别靠 magic 而不是长度猜测——v2 包尾 24B 视图套 32B 解析
   时 magic 必然错位不匹配，回退路径天然安全。
4. **壁纸走"分区内嵌"而不是"文件落盘"**：壁纸随 MIEDUNIN 块嵌进
   卸载器 EXE 尾部（与清单/图标同路径），卸载器自删除时壁纸随之
   消失，不留残余文件；渲染时全窗铺满 + 黑色半透明遮罩（alpha 120）
   保证深浅色壁纸下文字都可读，窗口 560×420 → 有壁纸时 800×540。
5. **grep 全量错误再次立功**：`read_uninstall_data` 改 3 元组后，
   install.rs 里旧版本检测处还有一个 2 元组解构调用（不在本轮改动
   视线内），Linux check 全量 grep `^error` 立即抓住。跨 crate 的
   函数签名变更必须全量编译验证，不能只信增量输出。

5. **grep 全量错误再次立功**：`read_uninstall_data` 改 3 元组后，
   install.rs 里旧版本检测处还有一个 2 元组解构调用（不在本轮改动
   视线内），Linux check 全量 grep `^error` 立即抓住。跨 crate 的
   函数签名变更必须全量编译验证，不能只信增量输出。

## 十四、落地清单 · 第十一批（NSIS 对齐 #10，v0.11.0）

**目录页空间动态刷新 + 安装目录校验**（步骤 20，用户缺陷报告驱动）。

1. **UI 状态要有"单一事实来源"**：`disk_free()` 此前查询 `self.dest`，
   但目录页用户编辑的是 `dir_text`——`dest` 只在点击「安装」后才更新，
   于是切换盘符后剩余空间永远冻结在默认盘。修复方式不是在多个事件
   处补刷新，而是引入 `current_dest()` 作为唯一口径（跟随 `dir_text`，
   空则回退默认），空间显示、Enter 校验、按钮禁用三处调用点同时受益。
   派生显示与编辑状态不同步是向导类 UI 的经典缺陷源。
2. **空间数字要跟着"路径语义"走，不只是"路径字符串"**：目录输入框里
   的值需要清洗（首尾空白/成对包裹引号/裸盘符 `C:` 规范化为 `C:\`），
   否则查询与写盘用的路径口径不一。清洗逻辑下沉
   `miedit_core::installdir::parse_dir_text` 纯函数并单测，UI 与
   start_install 共用，杜绝"显示一套、安装一套"。
3. **不存在盘符不能误报成 C 盘**：`disk_free_space` 原来在路径无任何
   已存在祖先目录时回退 `C:\` 查询——对不存在的盘符 Q:\foo，用户会
   看到 C 盘的空间，属于"看似成功实则错误"的最坏类别。改为返回 None
   （UI 隐藏该行），配合 `drive_root_of(..).exists()` 校验把不存在盘符
   直接判无效并禁用前进。
4. **NSIS 默认即约束**：`AllowRootDirInstall` 默认禁止盘根/网络共享根
   安装，MIEdit 此前对根目录毫无约束。补齐 `allow_root_dir_install`
   （默认 false）时保持同序：默认值即 NSIS 默认；确有根目录安装需求
   的作者显式打开开关。UNC 共享根 `\\srv\share` 与盘根同判（NSIS
   "locally into a network share" 同样算根），避免把共享根当普通目录。
5. **校验要"禁用 + 提示"而非"点击后弹窗"**：延续空间不足时的既有
   模式（按钮禁用 + 红色提示行），目录非法/根目录拒绝也走同一模式，
   每次输入变化实时重算。比 NSIS 的"点 Next 才 MessageBox"更顺滑，
   但判定口径（何时可前进）与 NSIS 语义一致。

## 十五、落地清单 · 第十二批（NSIS 对齐 #11，v0.12.0）

**安装导航 + UI 材质/模板 + Linux/CLI 适配 + 组件空间联动**（步骤 21，
七项用户指令一次交付）。

1. **回归测试要复现用户原话的场景**：用户说"默认 D:/Demo 改成
   E:/Demo 需立即刷新剩余空间"——修复已在上一轮（current_dest 跟随
   dir_text），这轮把该场景固化为单测（正/反斜杠两条链路逐段断言：
   清洗 → 盘根提取 → 校验放行）。用户的描述就是最好的测试用例名。
2. **玻璃材质在"构造期固化"而不是"渲染期分支"**：液态玻璃要求输入
   框/描边用半透明高光色——如果把分支散在每个 Frame 调用点会改十几
   处；改为 Theme::from_config 构造时直接产出玻璃变体色（field/
   card_stroke 固化为 rgba 白），渲染点零改动。主题系统把"材质决策"
   收敛在构造一处。
3. **模板 = 整体接管调色板**：五套模板若与 ui.accent 逐字段混合，会
   产出"半新半旧"的缝合观感；规则定为 `ui.template` 非空时整体接管
   （含 accent），空串才用显式色。风格预设宁全勿混。
4. **跨平台的"同包异构"**：同一个安装包在 Windows 内嵌 PE 卸载器
   （MIEDUNIN footer），在 Linux 无法执行 PE——finalize 按平台分流：
   Windows 内嵌、Linux 落盘 uninstall.json（卸载器本来就有清单文件
   回退，恰好构成完整的 Linux 卸载闭环）。平台差异收敛在 finalize
   一处，清单结构两端一致。
5. **Windows 风格脚本跨平台可用**：Linux 路径模板把 `\` 统一转 `/`，
   Windows 下写的 `'{autopf}\{app_name}'` 在 Linux 同样解析成
   `/opt/App`——作者心智模型不变，平台差异由解析层吸收。
6. **输出管道会吞 `[m`**：排查 CI"损坏表达式"时，工具输出显示
   `$(ath]::Round`，hexdump 后发现文件里始终是正确的 `$([math]`——
   显示管道把 `[m` 当 ANSI 序列剥掉了。验证文件内容必须用 od/hexdump，
   不能只信打印回显；"看起来坏了"不等于"真的坏了"。

## 十六、落地清单 · 第十三批（NSIS 对齐 #12，v0.13.0）

**注册表全类型与精确卸载清理 + ExecWait 自定义命令 + 组件分组树**
（步骤 22–24）。

1. **既有键不可整树删，精确清理靠"新建标记"**：旧实现把自定义注册表键
   记进 `registry_keys`，卸载时整树删除——写入既有键（如
   `HKCU\Software\Classes\.myext`）会把用户安装前的内容一起删光。
   修复不是"删得更小心"，而是**在安装时多记一个事实**（键是否为本次
   新建，`registry_key_exists` 判断）：卸载时新建键才整树删，既有键只删
   本次写入的值。清理策略的正确性必须在写入时留下证据，不能靠删除时猜测。
2. **命名冲突要查全仓库再定字段名**：新加"安装后命令"字段时顺手叫
   `run_after`，编译才发现与完成页运行程序的旧字段 `run_after: String`
   撞名——既有字段承载旧脚本兼容，绝不能动，新字段改名 `exec_*`
   （恰好贴合 NSIS Exec 语义）。加字段前先 grep 一次全部结构体字面量，
   `..Default::default()` 是给 builder 这类逐字段构造点的保险。
3. **依赖库的枚举名要对着本地源码写**：winreg 0.52 的 RegType 变体是
   Windows API 风格 `REG_EXPAND_SZ`（宏展开），不是文档常见的
   `RegExpandSz`；且 `Vec<u8>` 没有 `ToRegValue` 实现——REG_BINARY/
   REG_EXPAND_SZ 必须走 `set_raw_value` 手工构造字节（UTF-16LE+NUL）。
   交叉编译报错后用 `grep ~/.cargo/registry/src/.../winreg-0.52*` 直接
   看真源码，比猜 API 快得多。
4. **折叠树的关键是"组行自身永远渲染"**：SectionGroup 树折叠时，折叠
   的组行本身还要显示（否则 ▸ 展开按钮消失，用户无法展开回来）。
   隐藏判定要区分"真祖先被折叠"（组行）与"整条组路径被折叠"（组件行），
   一个 `rsplit_once('/')` 取父路径的差别就是交互是否可恢复的分界。
5. **卸载侧的命令失败要"记录而不中止"**：卸载前命令（停服务/关进程）
   失败就中止卸载会让应用永远删不掉；失败仅记入错误清单继续删除，
   被占用文件本来就有重试/重启删除路径兜底。安装侧则相反——
   `exec_after` 失败中止但要保证清单/卸载器已落盘（失败也可卸载）。
   同一个特性在两个生命周期里的容错方向相反，取决于"失败后用户还能
   不能回到干净状态"。

## 十七、落地清单 · 第十四批（NSIS 对齐 #13，0.1.0）

**压缩格式扩充（LZMA/LZMA-solid/Brotli/Brotli-solid）+ 版本号固定 0.1.0**
（步骤 25，用户指令驱动）。

1. **"纯 Rust 无编码器"的阻塞是动态的，要定期重查生态**：步骤 16 阻塞时
   评估的只有 lzma-rs（编码器是存根）；两周后 lzma-rust 0.1.7（sevenz-rust
   同源，tukaani xz-for-java 移植）已带可用 encoder。阻塞项要写明复查
   关键词与候选项目，每轮花 30 秒重查，而不是永久搁置——本次 NSIS 旗舰
   lzma-solid 因此从"待成熟"变成"已落地"。
2. **压缩格式的向后兼容 = 标志位锚点测试**：bit0/bit1 是历史包格式的
   契约，新增算法只能加位（bit2=brotli、bit3=lzma）不能动位；用常量
   断言单测锁住（FLAG_BROTLI==4、FLAG_LZMA==8、from_flag 组合位拒绝），
   比口头注释可靠。未知标志位要报错并提示"需要新版本安装器"，
   与 NSIS 一致：lzma 包需要带 lzma 支持的 stub。
3. **自解压模型的 e2e 验证必须运行"合体产物"**：安装器按 NSIS 自解压
   模型用 `current_exe()` 读自身包数据——拿裸 stub 加包路径参数会报
   "未找到包尾标记"，不是包坏了，是运行对象错了。验证自解压 EXE 必须
   直接运行编译产物（NSIS 同理：装的是 setup.exe 而非 stub）。
4. **新增压缩档位要用"体积排序"证明价值**：不能只是"能跑"。重复文本
   对照单测（brotli/lzma 不劣于 deflate）+ 实机 6 档端到端体积排序
   （lzma-solid 最小）双重锁定——后者恰好与 NSIS 默认 lzma-solid 的
   官方选择互证，说明算法档位对齐是真实等价而非形似。
5. **压缩参数宁取"质量/耗时平衡"不追极限**：Brotli quality 9 而非 11
   （11 收益有限但耗时陡增）、LZMA preset 6（NSIS 默认档位，8MB 字典）。
   安装包编译是交互式操作，压缩耗时是用户体验的一部分；flate2 best()
   的先例说明本项目一致采用"最好但可接受"档位。
