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
   不合格依赖。（后续：2026-11-20 #22 命中 `mzstd`/`libbz2-rs` 后已解阻，
   见路线图步骤 16 解阻记录与本文二十六节。）


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
   lzma-solid 因此从"待成熟"变成"已落地"。复查机制再次验证（2026-11-20
   #22）：zstd 侧命中 `mzstd`（zstd 1.5.7 编码器逐字节等价纯 Rust 移植），
   bzip2 侧命中 `libbz2-rs-sys`（libbzip2 纯 Rust 化，bzip2 0.6 默认后端），
   步骤 16 两个遗留阻塞项同轮全部解阻——定期复查比"等成熟再回看"可靠。
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

## 十八、落地清单 · 第十五批（NSIS 对齐 #14，0.1.0）

**卸载器多语言（步骤 26）+ 环境变量/PATH 管理（步骤 27）**。

1. **"卸载器多语言"的回退语义要分新旧两个世界**：旧清单无 `languages`
   字段时若走安装器解析链（英文默认兜底），所有旧包的卸载器会突然变
   英文——这是行为回归。正确做法是按"清单是否携带语言包"分两条链：
   旧清单回退内置 zh-CN（保持历史中文硬编码观感），新清单与安装器同链
   （语言跟随安装语言，NSIS 语义）。serde default 的"缺字段"既是兼容
   也是信号，用 `languages.is_empty()` 区分新旧世界。
2. **环境变量卸载清理有两种正确答案，取决于写入方式**：PATH 式合并
   写入（append/prepend）卸载时**只移除自己写入的段**——整值还原会
   误删其他软件后来写入的段（A 装 → B 装 → 卸 A → B 的段丢失）；
   `all` 直接设值则记录**安装前原值**还原（或新建则删值）。清理引用
   结构（`EnvCleanupRef`）因此是"两种模式二选一"而不是统一字段。
3. **环境变量广播是"免注销"体验的关键**：NSIS WriteEnvStr 惯例在写完
   HKCU/HKLM 环境区后必须 `SendMessageTimeoutW(HWND_BROADCAST,
   WM_SETTINGCHANGE, "Environment")`，否则新值要等注销/重启才对
   Explorer 生效——用户会以为安装没生效。用 SMTO_ABORTIFHUNG + 2s
   超时，挂起的窗口不阻塞安装。
4. **winreg 0.52 的类型边界要提前摸清**：无 `to_multi_string` 解码
   REG_MULTI_SZ（手工 UTF-16LE 双 NUL 切分）、`create_subkey` 返回
   `(RegKey, RegDisposition)` 元组而非 `RegKey`（or_else 链会类型不匹）、
   `RegValue::to_string()` 可直接解 REG_SZ/EXPAND_SZ。平台 API 的
   返回值形状先查签名再组链式调用，能省一轮交叉编译试错。
5. **取消按钮的副作用要回到"未操作"状态**：卸载确认页取消时旧版调用了
   `cleanup_after_uninstall`（删清单 + 调度自删除）——用户放弃卸载后
   安装反而永久失去卸载能力。对照 NSIS：取消 = 中止向导，一切保持
   安装后原状。只有真正执行过卸载的页面（详情/完成/失败）才允许收尾
   自删除。

## 十九、落地清单 · 第十六批（NSIS 对齐 #15，0.1.0）

**卸载语言跟随安装（步骤 28）+ 卸载清单 NSIS 处理语义（用户指令驱动）**。

1. **"语言页"不等于"语言跟随"——NSIS 的正解是持久化安装语言**：MUI
   `MUI_UNPAGE_LANGUAGE` 只解决"装的时候选什么"；卸载器自己再弹一次
   语言页反而是打扰（NSIS 官方多语言模板的惯例是 `un.onInit` 从
   注册表/INF 读回安装语言）。MIEdit 的落点：安装器 finalize 时把
   **实际选定**的语言 id 写进卸载清单 `language` 字段，卸载器按
   精确 → 主标签匹配直接使用，全程零提问。单语言包同样要记录——
   "只有一个语言"和"没记录语言"在卸载器侧是不同的回退分支。
2. **字段级兼容要给旧数据留"信号位"**：新清单有 `language`、旧清单
   没有——serde default 给空串，这既是兼容也是**信号**：卸载器用
   `language` 是否为空区分新旧世界（新清单永不显示语言选择页；旧清单
   多语言仍显示，行为不回归）。与 #14 的 `languages.is_empty()` 分链
   是同一手法：清单缺字段本身承载"哪一版行为"的信息。
3. **卸载顺序是语义不是实现细节**：NSIS UninstallLog 惯例按安装**相反
   顺序**删除（最后装入的先删——补丁覆盖的文件先于原始文件消失，目录
   自底向上变空）；`RMDir` 只删**空目录**，非空保留（用户放的文件不是
   错误）。MIEdit 旧版把"目录非空"记为卸载失败——NSIS 视角这是正常
   结局（清单外内容绝不碰）。改为保留 + 日志提示，卸载统计不再虚报失败。
4. **路径拼接交给 Path，不要手写字符串拼接**：卸载目录处理原来是
   `install_dir.trim_end_matches(...) + "\\" + d`——Windows 专用写法，
   Linux 卸载（uninstall.json 路径）直接拼坏。`Path::join` 一行替代，
   分隔符由平台决定；这也是"清单处理语义"改造时顺带修复的典型：动
   旧代码前先看它在哪个平台会坏。
5. **语言匹配函数要与启动预选同口径**：卸载器读回语言用精确 → 主标签
   两级匹配，与安装器 `initial_language` 的匹配规则完全一致（同一
   `lang_matches` 底座）——否则会出现"装的时候按主标签匹配到了
   zh-CN，卸载时按精确匹配找不到"的不对称回退。抽成
   `langs::match_language_index` 纯函数加单测，两处共用一份规则。

## 二十、落地清单 · 第十七批（NSIS 对齐 #16，0.1.0）

**warning 清零（步骤 31）+ MI 脚本 .mi/VSCode 式 IDE 页/导航栏（步骤 29）
+ 安装包打开速度优化（步骤 30）**。

1. **warning 是债，尤其是"未来硬错误"类**：rustc 1.100 对 `Stroke::new(1.0, ...)`
   的浮点字面量回退（`float_literal_f32_fallback`）给出"将在未来版本成为
   硬错误"的警告——46 处积压在三个 GUI crate。教训：机械类 warning 用
   模式化替换一次清零（`_f32` 后缀），不要留到 CI 升 rustc 时爆雷；清零
   后 build 与 test 两套 target 都要验证（test target 有独立的未用导入面）。
2. **"语法高亮"不必引入编辑器组件**：egui `TextEdit::layouter` 允许接管
   文本布局——在同一 LayoutJob 里先画行号段（dim 色、等宽对齐）再画
   语法分色段，天然对齐、无独立 gutter 滚动同步问题、编辑缓冲不受影响。
   轻量 IDE 的正确打开方式是"布局层做文章"，不是换组件。手写 TOML 词法
   着色器保持纯函数（`mi_script_job`），字符串内 `#` 误判等边界直接单测锁定。
3. **速度优化的第一步是给启动路径做减法**：安装器打开慢的主因不是窗口
   初始化，而是启动时同步做的两件事——全包 CRC32（线性于包体积：256MB
   包 637ms）与大图壁纸 PNG 解码。把"校验"改成"提取前必须完成"（CrcGate
   + Condvar）而不是"删掉校验"，NSIS CRCCheck 语义无损；把"壁纸"改成
   "先渐变后壁纸"（渐进式呈现），与网页图片加载同 UX 哲学。
4. **e2e 篡改测试要先核对字节偏移落在被校验分区**：第一次把 payload
   字节翻转载于 `file_size - 132 - 5KB` 处——落在卸载器内嵌数据区（MIEDUNIN
   footer 的壁纸字节，无 CRC 覆盖），静默安装 exit 0，差点误判"CRC 失效"。
   用探针打印 Trailer 各分区 off/len 后才知道 payload 在文件中部（stub
   ELF 之后）。篡改检测类测试必须以分区真实范围为准，不能用"离结尾多远"
   猜测布局。
5. **静默与 GUI 的校验时机可以不同，语义必须相同**：静默模式保持同步
   CRC（自动化需要"装完即确定性成功"的退出码），GUI 模式后台校验
   （窗口先开，提取前把关）。同一个 CrcGate 两种用法：`ready(Ok)`
   直通与 `new_pending()` 等待——语义一致性靠"提取前 wait_ok"这一
   单一收口点保证，而不是靠两条路径各自约定。

## 二十一、落地清单 · 第十八批（NSIS 对齐 #17，0.1.0）

**脚本编辑器深度对齐 VSCode（步骤 32）**——查找替换/命令面板/转到行/
自动缩进/括号补全/minimap/面包屑/状态栏/高亮层/Dark+ 配色。

1. **覆盖式高亮层胜过布局层手术**：在 LayoutJob 里把查找命中/当前行/错误行
   做进语法分色段，需要把 token 流与命中区间做区间切割合并——三层叠加时代码
   复杂度爆炸。改为"正常布局 + 事后 galley 几何覆盖"：`pos_from_ccursor`
   逐字符定位画半透明矩形，文本、行号、高亮三者天然同步。代价是覆盖层画在
   文字之上（半透明可接受），换来的是零切割逻辑。VSCode 本身的 selection
   渲染也是同一思路（先矩形后文字）。
2. **行号内嵌布局时，galley 字符索引 ≠ 文本字符索引**：行号前缀（`"  1  "`）
   也是 galley 内容，每行多 `w+2` 个字符——文本字符 c 在 galley 里的索引是
   `c + (行号+1)*前缀宽`。漏掉这个偏移，高亮矩形会整体漂移；行尾换行符
   归属哪一行、文档无末尾换行时行首表无哨兵，都是踩过的坑，全部用
   `line_start_chars`（按字符流扫描）+ 单测锁定。
3. **字符索引与字节索引的换算必须写在纯函数里**：TOML 脚本满是中文键值，
   查找返回字符区间、`String::replace_range` 吃字节区间——`char_range_to_byte_range`
   第一版在"break 后又执行 `if ci == end` 尾部修正"处踩坑：break 时 ci==end
   也成立，`be` 被二次覆盖成 text.len()，替换吞掉整串（测试当场抓住）。
   教训：区间换算函数写成"找到即 return"的单出口形式，配 CJK 偏移单测。
4. **egui 后处理式"自动编辑"是轻量 IDE 的可行路径**：自动缩进/Tab 转空格/
   括号补全都用同一套管线——TextEdit 展示前从 `ctx.input` 采集编辑意图
   （Enter/Tab/Text 事件），展示后若编辑器持焦点，就按意图对文本做纯函数
   变换，再 `TextEditState::set_char_range` 回写光标并 store。不必 fork
   TextEdit，也不必拦截事件流；纯函数部分全部单测。风险点是"光标停在刚
   插入字符之后"的约定（caret-1 处是 TextEdit 刚插入的字符），测试按此
   语义书写。
5. **命令面板/查找浮层用"动作枚举"解耦**：小部件只产出 `IdeAction`，
   `handle_ide_action` 统一执行——菜单栏、工具栏、命令面板、查找小部件、
   minimap 点击五条入口共用一个动作出口，避免每处复制一遍业务逻辑；
   egui 嵌套菜单闭包的可变借用冲突用 `RefCell<Option<IdeAction>>` 收集
   解决（closure 链无法直接共享 &mut）。
6. **Esc 是分层资源**：命令面板、转到行、查找浮层同时打开时，Esc 应逐层
   关闭而非直接关窗口——页面级先判面板开合再判查找开合，向导级 Enter/Esc
   拦截在脚本页让位（多行编辑器需要这两个键）。快捷键全集
   （Ctrl+F/H/G/S、Alt+Z、Ctrl+Shift+P）只在脚本页生效，避免污染向导页。

## 二十二、落地清单 · 第十九批（NSIS 对齐 #18，0.1.0）

**MI 脚本 v2 指令层（步骤 33）+ 单实例互斥（步骤 34）+ `/LANG=`
语言覆盖（步骤 35）**——NSIS Section 指令语义 / CreateMutex 惯例 /
十进制 LCID 语义。

1. **TOML 数组表不能按名点路径引用元素**：`[[components]]` 是数组表
   （AoT），`[[components."核心".code]]` 不会把 code 挂到"核心"组件上，
   而是给最后一个元素挂一个名为"核心"的未知子表——serde 无
   `deny_unknown_fields` 时**静默忽略**，指令一个都不执行（端到端
   测试当场抓住：`logs` 目录没创建）。教训：AoT 元素的子数据必须写
   内联数组（`code = [{ op = "create_dir", path = "..." }]`），并且
   任何"配置字段是否真的解析到了"都要有端到端验证兜底，单看
   toml 解析成功不算数。
2. **NSIS 脚本的 `\` 路径在 Linux 是字面字符**：`$INSTDIR\logs` 展开后
   在 Linux 会创建名为 `inst\logs` 的目录——解包链路早有
   `entry.path.replace('\\', "/")` 先例，指令层漏了同一归一化。
   统一口径：文件系统路径一律 `replace('\\', MAIN_SEPARATOR)`，但
   注册表键的 `\` 是 hive 分隔符**绝不能归一化**——归一化必须按
   数据类型分域，不能一刀切。
3. **指令失败语义 = NSIS error-flag，不是 fail-fast**：NSIS 的
   `CreateDirectory`/`Delete`/`WriteReg` 失败只置 error flag 继续
   跑完 Section，脚本作者用 `IfErrors` 自行决定；我们的指令层照搬
   ——失败记警告继续，唯一例外是 `run` 的 `abort_on_error`（显式
   选择的中止开关，与 `[[install.exec_before]]` 契约一致）。声明式
   配置（`[[registry]]`）失败跳过同样是这个传统，两套语义已统一。
4. **指令副作用要回流卸载清单，NSIS 靠脚本自律，我们靠记录**：NSIS
   卸载 Section 手写反向操作；MI 脚本没有 un Section（v2 范围），
   所以指令写注册表/建目录必须登记进清单（`CodeEffects` →
   `registry_values` 精确清理 + `extra_dirs` 仅空删除）——把
   "脚本自律"换成"运行时记账"，卸载完整性不依赖作者小心。同类
   手法之前是 `RegistryValueRef`/`EnvCleanupRef`，现在是第三处。
5. **单实例互斥用 `Global\` 而非 `Local\`**：per-machine 安装器可能
   被不同用户会话先后触发，`Local\` 只在同会话生效；互斥名是
   `Global\MIEDIT-<app_id>[-uninstall]`，app_id 里的非法字符
   （`\`、空格等）替换为 `_`——互斥名允许任意字符但反斜杠是命名
   空间分隔符，必须清洗。非 Windows 平台恒成功（stub），行为差异
   写进函数文档而非 cfg 抖逻辑。
6. **NSIS `/LANG` 的值是十进制 LCID，不是语言 id**：对 Windows 用户
   `installer.exe /LANG=2052` 是肌肉记忆——复用系统语言检测已有的
   LANGIDS 表做反查（1033→en-US→主标签匹配语言包），再叠 `0x`
   十六进制与语言 id 两种写法；无效值宽容回退（NSIS 不因 /LANG
   无效而拒绝安装），参数解析与语言匹配分两层、各配单测。

## 二十三、落地清单 · 第二十批（NSIS 对齐 #19，0.1.0）

**UAC 执行级别清单（步骤 36）+ MI 指令扩充（步骤 37）+ 安装期磁盘
空间预检（步骤 38）**——NSIS `RequestExecutionLevel` /
`Rename`/`CopyFiles`/`WriteINIStr`/`Sleep` / 磁盘满中止语义。

1. **自研安装器不能指望系统"认得"你**：NSIS 安装器不写
   `RequestExecutionLevel`（默认 none）也能正常弹 UAC，是因为
   Windows 对历史 NSIS 安装器有启发式识别（installer detector）；
   自研安装器没有这个待遇，空清单会被按普通程序对待。MIEdit 的
   取舍是缺省显式声明（per_machine → requireAdministrator，按用户
   → asInvoker）——把"系统猜"换成"我们自己说"，行为可预期。
   `highest`（highestAvailable）三档照收，脚本写法同时接受 NSIS
   关键字与 manifest 原生取值两种别名。
2. **manifest 资源三件套同事务写**：图标/VERSIONINFO/RT_MANIFEST
   在同一 `UpdateResourceW` 事务里完成（复用步骤 6 的链路），
   `RT_MANIFEST` 是资源类型 ID 24——windows-rs 未绑定该常量，
   `MAKEINTRESOURCEW(24)` 的本质就是整数值当指针（项目里
   `make_int_res` 助手）。supportedOS 五档 GUID 以微软文档为准
   （Win8 是 `4a2f28e3-…`，凭记忆写成 `4a2f28e8-` 会静默失效）；
   DPI/长路径感知顺路写进同一清单（NSIS `ManifestDPIAware` 惯例）。
3. **WriteINIStr 的语义细节在"不区分大小写"**：Windows INI API
   节名/键名匹配都不区分大小写，但替换写入时保留**调用方的键名
   写法**（不是沿用文件里的旧写法）；键已存在原位替换、不存在
   插入节内末尾（下个节头前）、节不存在追加文件末尾、文件缺失
   当作空文件新建。逻辑写成 `micmd::write_ini_str` 纯函数（内容进
   内容出），磁盘读写留给执行器——五场景单测把边界全钉住。
4. **CopyFiles 的自嵌套要双层防护**：`copy_files from=$INSTDIR
   to=$INSTDIR\sub` 会把目标复制进自己再复制自己，无限膨胀。
   静态校验（micmd::validate，归一化路径前缀检查）在构建器问题
   面板提前报错；运行期 `copy_tree` 再用 canonicalize 防护一次
   （静态校验看不见变量展开后的真实路径）。NSIS 对此也是报错
   语义，双重检查不冲突。
5. **磁盘满用预检而不是等写失败**：NSIS 是写失败才报错（abort/
   retry 对话框），但写一半失败会留下半截文件；写盘前用
   `GetDiskFreeSpaceExW` 预检（`free + 旧文件占用 < 新文件 + 8MB
   余量` → 中止），覆盖写抵扣旧文件占位避免升级场景误报，剩余
   空间不可知（None）时跳过不误报。判定进 `files::is_space_insufficient`
   纯函数（saturating 算术防 u64 溢出），边界（恰好相等=足够）、
   抵扣、溢出全部单测钉住。
6. **指令创建的文件不自动回收是 NSIS 语义，不是缺陷**：端到端
   卸载验证发现 `write_ini`/`copy_files` 的产物留在盘上——查
   NSIS 行为一致：只有 `File` 安装的文件和 `WriteReg`/注册表类
   有卸载链路，指令产物要脚本作者用 un Section 手工清理。MI 脚本
   v2 没有 un Section，与 #18 教训 4（运行时记账）对齐的后续方向
   是把 write_ini/copy/rename 的产物也登记进清单——本轮先保持
   NSIS 同语义，记账升级留给下一批。

## 二十四、落地清单 · 第二十一批（NSIS 对齐 #20，0.1.0）

**MI 指令注册表全类型（步骤 39）+ ExecShell/RMDir（步骤 39）+
条件指令 IfFileExists/Abort（步骤 40）**——自研 `.mi` 语言指令集
扩至 13 条，控制流首次进入脚本层。

1. **snake_case serde 标签会"吃掉"驼峰连写**：`RmDir` 变体在
   `#[serde(tag = "op", rename_all = "snake_case")]` 下序列化为
   `rm_dir` 而不是 `rmdir`——NSIS 关键字是连写的（RMDir），
   MI 指令名也按连写设计。发现于 serde 往返单测（断言
   `"op":"rmdir"` 失败），解法是变体级显式
   `#[serde(rename = "rmdir")]`。同理 IfFileExists →
   `if_file_exists` 是期望行为（NSIS IfFileExists 也是分词）。
   教训：**指令名是语言对外契约，不能交给命名规则推导**。
2. **Rust 关键字字段用显式 rename 而不是 raw identifier**：
   `if_file_exists` 的 else 分支字段名是关键字 `else`。可用
   `r#else` raw identifier（serde 会正确序列化为 `"else"`），
   但显式 `#[serde(rename = "else")]` + Rust 字段名
   `else_branch` 更直白——字段名在 Rust 代码里可读（then/else
   不对称一眼可见），且规避 serde derive 对 raw ident 的
   边缘行为。两法皆可，选直白者。
3. **SetRegView 按"每指令内联"而不是全局切换**：NSIS 的
   `SetRegView` 是全局状态（切换后所有注册表指令都重定向，
   用完要手动切回），脚本作者忘切回是经典 bug 源。MI 脚本把
   视图做成 `write_reg` 的 `view` 字段（每指令独立声明），无
   跨指令副作用可"忘记"——同一脚本里 32/64 视图交错写互不
   干扰。这是对 NSIS 语义的**有意偏离**（更安全），文档要写明。
4. **递归块执行的上下文要收拢成结构体**：指令执行函数原来
   有 8 个参数（`#[allow(clippy::too_many_arguments)]`），
   `if_file_exists` 要把同一套上下文递归传给分支块——再加
   depth 就是 9 个。重构为 `InstCtx` 结构体（dest/vars/effects/
   dirs/seen_dirs/st/comp_name/repaint）后，`execute_block`
   递归只需 `(code, depth, ctx)` 三参，DetailPrint/日志/
   error-flag 语义从组件循环和分支两处收敛为单点维护。
   教训：**加第二个递归调用点之前先把参数包起来**。
5. **error-flag 语义在分支块里要沿用到顶层**：NSIS 分支内的
   指令失败同样只置 error flag 不中止（除非显式 Abort）。MI
   实现 `execute_block` 对 then/else 与组件顶层共用同一错误
   处理（失败记警告继续；`run.abort_on_error` 与 `abort`
   中止）——`exec_shell` 在无 xdg-open 的 Linux 沙箱失败，
   安装继续 exit 0，端到端日志可直接验证该语义。
6. **嵌套深度上限要静态+运行时双层防护**：`MAX_CODE_DEPTH = 4`
   在 `validate` 递归校验（构建器问题面板提前报错，错误位置带
   `then>else` 分支标签）与安装器 `execute_block`（depth 参数
   防御）各检查一次——serde 反序列化本身不递归爆炸（数据结构
   有限），但恶意构造的手写脚本可能绕过构建器直接进安装器，
   运行时防御不可省。验证边界：恰好 MAX 层合法、MAX+1 层报错，
   单测钉死。

## 二十五、落地清单 · 第二十二批（NSIS 对齐 #21，0.1.0）

1. **诊断器不能用严格类型解析做前置门**：`toml::from_str::<Config>`
   遇到未知 op / 缺字段直接失败——而这恰是诊断的主要对象（半成品
   脚本）。改为 `toml::Value` 宽松解析保语法，再逐条
   `serde_json::from_value::<MiInstruction>` 转换：错误消息自带
   `unknown variant` / `missing field`，直接利用。探针还发现 toml crate
   对 tag 枚举报 `invalid key` 而不透传 serde 变体错误（顶层内联表甚至
   是非法 TOML 文档）——**测试要走与生产一致的 Value→JSON 链路**，
   不能拿 `toml::from_str::<Enum>` 直接断言消息。
2. **嵌套分支的行号映射用"消费式行索引"**：同一行可能叠着外层与内层
   指令（单行 `if_file_exists ... then = [{ op = "run" }]`）。为每条
   指令分配独立行既不可能也无必要：`LineIndex.find_line` 优先匹配
   `op_of(line) == op` 的未消费行，命中即标记；找不到回退父行
   （hint）。诊断行号是"就近近似"，修复按钮只对
   `op_of(line) == 自身 op` 的行给出——避免把嵌套字段补到外层。
3. **修复按钮的载体是"整行替换"**：逐字符补丁（插入位置 + 长度）在
   编辑器并发编辑下容易错位；`QuickFix { line, new_line }` 整行替换
   语义简单、可撤销（编辑器自身 undo），越界时纯函数原样返回。代价
   是多行条目不给修复——单行条目覆盖绝大多数真实场景，够用。
4. **主题色不要用 const，用"调色板 + 原子开关"**：构建器 77 处
   `CARD/TEXT` 硬编码常量没法直接换肤。做法：`Palette` 结构体双套
   （DARK/LIGHT）+ `DARK_MODE: AtomicBool` + `pal()` 每帧取当前板；
   机械替换 `CARD` → `pal().card`。两个坑：① use 语句里的常量名会被
   一起替换破坏 import——替换后要修 use；② 模拟安装器的预览页要
   **解耦**成私有常量（安装器窗口视觉恒定深色，不随构建器主题）。
5. **菜单栏全局化是"冲突排查"的典型收益**：脚本页内嵌菜单栏（文件/
   编辑/查看/工具/帮助）与"IDE 顶部 文件/选项/设置/关于"指令冲突——
   双菜单并存既重复又挤占编辑区。把菜单上移为全局 TopBottomPanel
   （所有向导页可见），脚本页仅保留快捷键；`Esc` 分层顺序（预览 →
   面板 → 转到行 → 查找）在 update() 入口先于向导 Enter/Esc 处理，
   避免预览打开时误触"取消安装向导"。

## 二十六、落地清单 · 第二十三批（NSIS 对齐 #22，0.1.0）

**压缩档大扩充：BZip2/Zstd 落地 + 激进压缩档**（步骤 16 解阻 + 45，
用户指令「再增加压缩方法，增加激进压缩方案 / 优化 zstd/bzip2 编码器」）。

1. **压缩格式扩展把"分派"下沉 core，三个调用点零重复**：编译器选算法/
   安装器解 solid/预览摘要显示文案三处原本各自 match 字符串——本轮收敛为
   `compression_params`（值→(solid, algo, ultra)）+ `compression_label`
   （值→文案）+ `COMPRESSION_OPTIONS`（下拉顺序）三个 core 函数，
   新增档位只改 core 一处。分派逻辑共享是"新增格式不漏改"的唯一保障。
2. **ultra 激进档不改格式，只改编码参数**：LZMA 头部自带 props/dict_size、
   zstd 帧自描述——同一解码路径无需区分标准/激进档，解码端零改动、
   旧安装器照常解。若 ultra 引入新容器格式，兼容矩阵会平方级膨胀；
   参数级激进把增量压到零。
3. **激进档的收益与语料规模正相关，默认档仍是标准档**：2.86MB 混合语料
   实测 lzma-ultra 与 lzma 打平（64MB 字典对 2.86MB 数据无从发挥）；
   ultra 的目标场景是 ≥ 字典量级的大包。UI 把 ultra 放在列表尾部而非
   置顶，文档诚实记录"小语料无增益"——避免用户误以为激进档永远更小。
4. **纯 Rust 压缩器的耗时差异要写进文档**：同语料 bzip2（libbz2-rs）
   压缩 7.0s vs zstd（mzstd）2.0s vs deflate 0.9s——纯 Rust 实现性能
   与 C 版差距不小，用户选档时要对耗时预期负责；README 特性表给出
   实测数字而非"支持"两个字。
5. **大 crate 在低内存 CI/沙箱的编译策略**：mzstd（SIMD 密集）双进程
   并行 + debuginfo 会打爆 3.9GB 内存（SIGKILL）；`-j 1` +
   `CARGO_PROFILE_DEV_DEBUG=0` 可过。这不是代码问题（Windows/GitHub
   Actions 内存充足），但验证流程要写明：低内存环境用串行 + 无
   debuginfo 跑测试，避免误判"依赖不可用"。

## 二十七、落地清单 · 第二十四批（NSIS 对齐 #23，0.1.0）

**压缩链路回归防护：差分基准入库 + 种子模糊往返 + 14 档端到端矩阵**
（步骤 46/47，健壮性对齐）。

1. **基准要"确定性语料"，不要随机抽样**：压缩率对比若每次用不同语料，
   表格就不可比；SplitMix64 种子驱动（文本/二进制/零段/重复块/随机五熵级）
   让任何平台任何时间生成逐字节一致的语料——回归时重跑表格 diff 即可发现
   格式/性能退化。随机性只放在 fuzz（模糊往返）里，且同样种子可复现。
2. **模糊往返的尺寸序列要覆盖边界**：空输入（0B）、单字节、块边界
   （7B/64B/333B）、跨字典窗口（4KB/16KB）各档都有独立代码路径
   （头部/字典初始化/块切分），固定尺寸序列轮转比纯随机尺寸更容易
   稳定命中边界——0 失败的记录才有"回归基线"价值。
3. **端到端矩阵发现了单测发现不了的缺陷**：`mis build` 在 Linux 上
   `File::create` 新建输出文件丢失 stub 可执行位（0644）——包体在
   Windows 上无此概念，单测也全过；只有"构建→直接执行安装"的 e2e
   链路才能暴露。修复方式是编解放层回授权限位（`#[cfg(unix)]`），
   顺带证明：**跨平台交付物必须有一条无头 e2e 常态化链路**，
   `scripts/e2e-matrix.sh` 从此成为压缩链路改动的标配回归。
4. **低磁盘沙箱跑 e2e 的工程细节**：debug stub 自带符号可达数百 MB，
   而 e2e 每档都内嵌完整 stub——strip 副本（仅去 debuginfo）+
   逐档清理是磁盘受限环境的两板斧；脚本保留 `--keep` 排查开关但
   默认不留包。真机/CI 磁盘充足时可忽略，脚本行为不变。
5. **基准表要诚实记录"档位差距拉不开"的场景**：96KB 语料上 zstd/lzma/
   brotli 差距 ±0.7%、ultra 与标准档持平——这不是测量错误而是语料
   规模的物理天花板（20KB 随机段是不可压缩下界，字典无从发挥）。
   文档同时给出小语料表（本文件）与大语料实测（#22 的 2.86MB），
   并注明选档依据是包体量——对比表格的价值在于"知道何时相信它"。

## 二十八、落地清单 · 第二十五批（NSIS 对齐 #24，0.1.0）

**卸载端到端矩阵 + 抗损坏输入 fuzz**（步骤 48/49，健壮性对齐）。

1. **e2e 断言链每延伸一段，就能抓到一批单测够不着的真缺陷**：卸载阶段
   接进 e2e 矩阵后，14/14 档全部暴露「目录残留」——清单 `manifest_path`
   在序列化**后**才回填（清单文件内恒空串，卸载器删不掉 uninstall.json）、
   安装器副产物 install.log 无人认领、Linux 自删除分支压根是 no-op。
   三个问题在 Windows 实机上都"看起来正常"（内嵌清单不需要该字段、
   cmd 自删除链路完好）。教训：**端到端断言的覆盖面 = 成熟度的下限**，
   装→卸→全清是一条完整闭环，断言到"目录消失"才算验完。
2. **自删除脚本独立成纯函数**：`self_delete_script_unix` 把 shell 片段
   构造从 IO 分支里抽出来，POSIX 单引号转义、空路径跳过、仅空目录
   删除（NSIS RMDir 语义）全部单测可断言。延迟删除的时序（sleep 2
   等进程退出）与 Windows 分支（ping -n 3）语义对齐；平台不对称
   （Windows `rd /s /q` 更激进 vs Unix `rmdir` 仅空）诚实记录——
   Windows 行为已上线不动，Unix 选保守侧。
3. **fuzz 的不变量要选"可机器判定的"**：抗损坏 fuzz 不判断"应该成功"
   （损坏输入的成功与否无法预知），只断言**不 panic**（Ok/Err 都接受）
   ——这才能放变异、换种子、跑大量案例而不产生假阳性。配套的确定性
   变异（截断/位翻转/清零/追加/空）保证失败可复现。
4. **长度字段是文件格式的第一攻击面**：归档 comp_len/orig_len 遇
   u64::MAX 级损坏值时 `vec![0u8; len]` 直接 OOM abort（CRC 被跳过时
   是不可恢复 DoS）。2 GiB 上限 + 优雅报错一条 if 就解决；教训是
   **所有"从输入读取的长度/计数"在分配前都要设上限**，这比 fuzz 更
   早拦住崩溃。NSIS 对 firstheader/条目长度同样逐项校验。
5. **低内存沙箱的矩阵复跑策略**：debug stub 百 MB 级、每档内嵌完整
   stub——strip 副本 + 逐档清理延续 #23 的做法；本轮验证磁盘富余时
   脚本无需改行为。e2e 脚本参数（`--no-build`/`--keep`）让"重建产物"
   与"复跑矩阵"解耦，cron 每轮只做后者省 6 分钟编译。
