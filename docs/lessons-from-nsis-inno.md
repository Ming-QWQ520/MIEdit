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

> 对齐路线图见 [nsis-alignment-roadmap.md](nsis-alignment-roadmap.md)，按步骤持续迭代。
