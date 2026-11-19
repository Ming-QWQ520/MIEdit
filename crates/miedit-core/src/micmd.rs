//! MI 脚本 v2 指令层（NSIS Section 指令语义，路线图步骤 33/39/40）。
//!
//! MI 脚本 v1 是声明式 TOML（文件/注册表/快捷方式等），v2 在每个组件
//! （NSIS `Section`）上叠加**指令式代码段**：组件被勾选后按脚本顺序执行
//! 其 `code` 指令（未勾选的组件指令不执行——NSIS Section 语义）。
//!
//! 指令集（对标 NSIS 核心指令）：
//! | MI 指令          | NSIS 等价          | 说明 |
//! |------------------|--------------------|------|
//! | `run`            | `Exec`/`ExecWait`  | 运行外部命令（复用 [`crate::runcmd`]） |
//! | `write_reg`      | `WriteRegStr`/`DWORD`/`Bin`/`ExpandStr`/`MultiSz`/`SetRegView` | 写注册表值（str/dword/bin/expand/multi_sz + 视图） |
//! | `create_dir`     | `CreateDirectory`  | 创建目录（含父目录） |
//! | `delete_file`    | `Delete`           | 删除文件（缺失静默跳过） |
//! | `rename`         | `Rename`           | 同盘重命名/移动（目标已存在则失败，NSIS 同语义） |
//! | `copy_files`     | `CopyFiles`        | 复制文件或整个目录树到目标 |
//! | `write_ini`      | `WriteINIStr`      | 写 INI 节键值（节/键不区分大小写，Windows API 同语义） |
//! | `sleep`          | `Sleep`            | 暂停指定毫秒 |
//! | `detail_print`   | `DetailPrint`      | 详情日志 |
//! | `exec_shell`     | `ExecShell`        | 系统动词打开目标（URL/文档；Linux 仅 open→xdg-open） |
//! | `rmdir`          | `RMDir`/`RMDir /r` | 删目录（默认仅删空；`recursive = true` 整树） |
//! | `if_file_exists` | `IfFileExists`     | 条件分支（then/else 嵌套指令块，深度 ≤ [`MAX_CODE_DEPTH`]） |
//! | `abort`          | `Abort`            | 中止安装（message 记入失败页/日志） |
//!
//! 路径与参数支持 NSIS 常量变量（`$INSTDIR`/`$DESKTOP`/`$SMPROGRAMS`…），
//! 安装期由 [`expand_vars`] 展开；[`standard_var_map`] 用真实 Shell 目录
//! 构建变量表。校验为纯函数 [`validate`]（递归校验嵌套块），编辑器与
//! 安装器共用。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// APPDATA 系环境变量缺失时的 XDG 回退（Linux 适配，步骤 21 延续）。
fn env_or_xdg(key: &str, home: &str, kind: &str) -> String {
    match std::env::var(key) {
        Ok(v) if !v.trim().is_empty() => v,
        _ => {
            if home.is_empty() {
                String::new()
            } else {
                PathBuf::from(home)
                    .join(".local")
                    .join(kind)
                    .display()
                    .to_string()
            }
        }
    }
}

/// 一条组件指令（组件表内的 `code` 内联数组，`op` 判别）。
///
/// `[[components]]` 是 TOML 数组表，指令以内联表数组挂在组件上：
/// ```toml
/// [[components]]
/// name = "Core"
/// files = [0]
/// code = [
///   { op = "run", cmd = "$INSTDIR\\tools\\reg.bat", abort_on_error = false },
///   { op = "write_reg", key = "HKCU\\Software\\Demo", name = "Path", value = "$INSTDIR" },
///   { op = "create_dir", path = "$INSTDIR\\logs" },
/// ]
/// ```
/// （注意：`[[components."Core".code]]` 是独立子表而非该组件的字段，
/// AoT 元素不能用点路径按名引用——必须使用内联数组形式。）
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum MiInstruction {
    /// NSIS `Exec`/`ExecWait`：运行外部命令（复用 `[[install.exec_before]]`
    /// 的执行器语义：`abort_on_error` 时非零退出码中止安装）。
    Run {
        /// 可执行文件路径（支持 NSIS 变量与 `{install_dir}` 模板）
        cmd: String,
        /// 参数串（引号保留空格）
        #[serde(default)]
        args: String,
        /// 工作目录（空 = 继承安装目录）
        #[serde(default)]
        working_dir: String,
        /// 非零退出码是否中止安装（NSIS ExecWait + IfErrors 惯例）
        #[serde(default)]
        abort_on_error: bool,
    },
    /// NSIS `WriteRegStr`/`WriteRegDWORD`/`WriteRegBin`/`WriteRegExpandStr`/
    /// `WriteRegMultiSz`：写单个注册表值（全类型，路线图步骤 39）。
    ///
    /// `type`（大小写不敏感，默认 `str`）：
    /// - `"str"`：REG_SZ；
    /// - `"dword"`：十进制 / `0x` 十六进制（负数按 32 位补码回绕）；
    /// - `"bin"`：REG_BINARY，十六进制串（空白可忽略，长度偶数，空 = 零长值）；
    /// - `"expand"`：REG_EXPAND_SZ，值内 `%VAR%` 由系统按引用展开；
    /// - `"multi_sz"`：REG_MULTI_SZ，`value` 用 `|` 分隔多段（如 `"a|b|c"`）。
    ///
    /// `view`（NSIS `SetRegView`，32/64 注册表重定向）：`""`/`default`（默认）/
    /// `"64"`/`"32"`——**每指令独立视图**，不产生跨指令的全局副作用
    /// （NSIS SetRegView 是全局切换，这里按指令内联更安全）。
    WriteReg {
        /// 含 hive 前缀的键，如 `HKCU\Software\Demo`
        key: String,
        /// 值名
        name: String,
        /// 值内容（str/expand 支持 NSIS 变量与 `{install_dir}` 模板）
        value: String,
        /// `"str"`（默认）/ `"dword"` / `"bin"` / `"expand"` / `"multi_sz"`
        #[serde(default)]
        r#type: String,
        /// 注册表视图：`""`/`default`/`64`/`32`（空 = default）
        #[serde(default)]
        view: String,
    },
    /// NSIS `CreateDirectory`：创建目录（含父目录；已存在则跳过）。
    CreateDir {
        /// 目录路径（支持 NSIS 变量）
        path: String,
    },
    /// NSIS `Delete`：删除文件（不存在时静默跳过，NSIS 同语义）。
    DeleteFile {
        /// 文件路径（支持 NSIS 变量）
        path: String,
    },
    /// NSIS `Rename`：同盘重命名/移动文件。目标已存在时失败
    /// （NSIS 同语义：不覆盖既有目标）；失败按 error flag 记警告继续。
    Rename {
        /// 源路径（支持 NSIS 变量）
        from: String,
        /// 目标路径（支持 NSIS 变量）
        to: String,
    },
    /// NSIS `CopyFiles`：复制文件或整个目录树到目标。源不存在报错
    /// （NSIS error flag 语义）；目标父目录自动创建，同名文件覆盖。
    CopyFiles {
        /// 源路径（文件或目录，支持 NSIS 变量）
        from: String,
        /// 目标路径（复制目录时为目标根，自动创建）
        to: String,
    },
    /// NSIS `WriteINIStr`：写 INI 文件节键值。节/键匹配不区分大小写
    /// （Windows `GetPrivateProfileString` 同语义）；文件不存在则创建。
    WriteIni {
        /// INI 文件路径（支持 NSIS 变量）
        file: String,
        /// 节名（不含方括号）
        section: String,
        /// 键名
        key: String,
        /// 值（支持 NSIS 变量）
        value: String,
    },
    /// NSIS `Sleep`：暂停指定毫秒（脚本式等待外部进程落盘等场景）。
    Sleep {
        /// 毫秒数（NSIS `Sleep 1000` 语义）
        ms: u64,
    },
    /// NSIS `DetailPrint`：向安装详情页/日志输出一行。
    DetailPrint {
        /// 日志文本（支持 NSIS 变量）
        text: String,
    },
    /// NSIS `ExecShell <verb> <target> [params]`：用系统 shell 动词打开目标
    /// （URL / 文档 / 可执行程序；如安装完成后打开说明页）。verb 常用
    /// `open`/`openas`/`print`/`runas`；空按 `open` 处理。Windows =
    /// `ShellExecuteW`；Linux 仅支持 `open`（`xdg-open`），其余动词报错
    /// （上层按 error-flag 记警告继续）。
    ExecShell {
        /// shell 动词（空 = `open`）
        #[serde(default)]
        verb: String,
        /// 目标（URL / 文件路径，支持 NSIS 变量）
        target: String,
        /// 参数串（引号保留空格，可空）
        #[serde(default)]
        args: String,
    },
    /// NSIS `RMDir` / `RMDir /r`：删除目录。
    ///
    /// - `recursive = false`（默认，NSIS `RMDir` 裸语义）：**仅删除空目录**，
    ///   非空目录静默保留记日志（不算失败）；目录缺失静默跳过。
    /// - `recursive = true`（NSIS `RMDir /r`）：整树递归删除。
    ///
    /// 与卸载器清单清理（[`crate::manifest`]）同语义，仅作用于安装期
    /// 脚本化清理（如清旧版本残留目录）；删除动作不登记卸载清单
    /// （被删的东西卸载时自然无需清理）。
    #[serde(rename = "rmdir")]
    RmDir {
        /// 目录路径（支持 NSIS 变量）
        path: String,
        /// `true` = `RMDir /r` 整树删除；默认仅删空目录
        #[serde(default)]
        recursive: bool,
    },
    /// NSIS `IfFileExists`：条件分支。`path` 存在执行 `then`，否则执行
    /// `else`（两者均可为空 = 什么都不做）。分支内指令按序执行，沿用
    /// 顶层 error-flag 语义（失败记警告继续；`run.abort_on_error` 与
    /// `abort` 仍中止安装）。嵌套深度上限 [`MAX_CODE_DEPTH`]（[`validate`]
    /// 静态拒绝 + 安装期运行时防御）。
    IfFileExists {
        /// 判定目标（文件或目录；支持 NSIS 变量）
        path: String,
        /// 存在时执行的指令块
        #[serde(default)]
        then: Vec<MiInstruction>,
        /// 不存在时执行的指令块（serde 字段名 `else`）
        #[serde(default, rename = "else")]
        else_branch: Vec<MiInstruction>,
    },
    /// NSIS `Abort`：中止安装。`message` 非空时作为中止原因记入日志
    /// （失败页/静默日志），空则用默认文案。安装器映射：中止 → 失败页
    /// （NSIS Abort → "Installation aborted"）。
    Abort {
        /// 中止原因（可空；支持 NSIS 变量）
        #[serde(default)]
        message: String,
    },
}

/// `if_file_exists` 嵌套深度上限（[`validate`] 与安装器运行时共用）。
/// NSIS 脚本几乎没有更深的分支嵌套；4 层足以覆盖真实场景且防递归爆炸。
pub const MAX_CODE_DEPTH: usize = 4;

/// 全部指令 op 名（serde tag 集合；诊断层 [`crate::midiag::KNOWN_OPS`]
/// 与编辑器补全/文档共用。`test_known_ops_all_parse` 守卫清单与
/// `MiInstruction` 反序列化一致，防止新增指令漏登记）。
pub const KNOWN_OPS: &[&str] = &[
    "run",
    "write_reg",
    "create_dir",
    "delete_file",
    "rename",
    "copy_files",
    "write_ini",
    "sleep",
    "detail_print",
    "exec_shell",
    "rmdir",
    "if_file_exists",
    "abort",
];

/// NSIS 常量变量集（[`standard_var_map`] 产出、[`expand_vars`] 消费）。
///
/// 目录语义对齐 NSIS shell folders 常量；`per_machine` 影响
/// `$DESKTOP`/`$SMPROGRAMS`/`$STARTMENU` 的公共/个人位置（NSIS
/// `SetShellVarContext all/current` 语义）。
pub struct MiVars<'a> {
    /// `$INSTDIR`：安装目录
    pub install_dir: &'a Path,
    /// `$EXEDIR`：安装器自身所在目录
    pub exe_dir: &'a Path,
    /// true = 全机安装（shell 目录取公共位置）
    pub per_machine: bool,
}

impl<'a> MiVars<'a> {
    /// 构建标准变量表（跨平台：Windows 用 winutil Shell 目录，Linux 回退
    /// XDG/环境变量——步骤 21 Linux 适配的延续）。
    pub fn standard_map(&self) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert("INSTDIR".into(), self.install_dir.display().to_string());
        m.insert("EXEDIR".into(), self.exe_dir.display().to_string());
        // Shell 目录（winutil 跨平台：Windows=SHGetFolderPath，Linux=XDG 回退）
        m.insert(
            "DESKTOP".into(),
            crate::winutil::desktop_dir(self.per_machine).display().to_string(),
        );
        m.insert(
            "SMPROGRAMS".into(),
            crate::winutil::start_menu_dir(self.per_machine)
                .display()
                .to_string(),
        );
        m.insert(
            "STARTMENU".into(),
            crate::winutil::start_menu_dir(self.per_machine)
                .display()
                .to_string(),
        );
        m.insert(
            "QUICKLAUNCH".into(),
            crate::winutil::quick_launch_dir().display().to_string(),
        );
        // 环境变量常量（NSIS $APPDATA 等语义；Linux 回退 XDG）
        let home = std::env::var("HOME").unwrap_or_default();
        m.insert(
            "APPDATA".into(),
            env_or_xdg("APPDATA", &home, "share"),
        );
        m.insert(
            "LOCALAPPDATA".into(),
            env_or_xdg("LOCALAPPDATA", &home, "share"),
        );
        m.insert(
            "TEMP".into(),
            std::env::var("TEMP")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .or_else(|| {
                    std::env::var("TMPDIR").ok().filter(|s| !s.trim().is_empty())
                })
                .unwrap_or_else(|| "/tmp".into()),
        );
        m.insert(
            "WINDIR".into(),
            std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into()),
        );
        m.insert(
            "SYSDIR".into(),
            format!(
                "{}\\System32",
                std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into())
            ),
        );
        m.insert(
            "PROGRAMFILES".into(),
            std::env::var("ProgramFiles").unwrap_or_else(|_| "C:\\Program Files".into()),
        );
        m
    }
}

/// NSIS 常量变量展开：`$INSTDIR\bin` → 真实路径。
///
/// 纯函数：`vars` 由调用方给出（测试确定性）；长名优先替换
/// （避免未来出现前缀重叠的变量名时短名误吞长名）。形如
/// `$UNKNOWN` 的未定义变量原样保留（NSIS 编译期会报错，安装期
/// 我们选择保守保留原样并记日志）。
pub fn expand_vars(input: &str, vars: &BTreeMap<String, String>) -> String {
    // 长名优先
    let mut names: Vec<&String> = vars.keys().collect();
    names.sort_by(|a, b| b.len().cmp(&a.len()));
    let mut out = input.to_string();
    for name in names {
        let pat = format!("${name}");
        if out.contains(&pat) {
            out = out.replace(&pat, vars.get(name).map(|s| s.as_str()).unwrap_or(""));
        }
    }
    out
}

/// 指令静态校验（编辑器问题面板与安装器共用；返回错误描述列表）。
///
/// 只做**安装期可静态判定**的检查：必填字段、dword/bin/multi_sz 数值
/// 合法性、注册表键 hive 前缀、注册表视图枚举、嵌套深度上限。变量展开
/// 后的路径是否存在属运行期，不在静态校验范围。递归校验
/// `if_file_exists` 分支块（深度 ≤ [`MAX_CODE_DEPTH`]）。
pub fn validate(code: &[MiInstruction]) -> Vec<String> {
    validate_at(code, 0, "")
}

/// 单条指令的非递归校验（诊断层 `midiag` 的 `if_file_exists` 分支
/// 遍历自控行号映射，因此需要无前缀、不递归的单条版本）。
/// 返回的错误描述**不带「指令 #N」前缀**，形如 `run.cmd 为空`。
pub fn validate_one(inst: &MiInstruction) -> Vec<String> {
    let mut errs = Vec::new();
    match inst {
        MiInstruction::Run { cmd, .. } => {
            if cmd.trim().is_empty() {
                errs.push("run.cmd 为空".into());
            }
        }
        MiInstruction::WriteReg { key, name, value, r#type, view } => {
            let k = key.trim();
            if k.is_empty() {
                errs.push("write_reg.key 为空".into());
            } else if !k.starts_with("HKLM\\") && !k.starts_with("HKCU\\") {
                errs.push(format!(
                    "write_reg.key 需以 HKLM\\ 或 HKCU\\ 开头: {k}"
                ));
            }
            if name.trim().is_empty() {
                errs.push("write_reg.name 为空".into());
            }
            let ty = r#type.trim().to_ascii_lowercase();
            match ty.as_str() {
                "" | "str" => {}
                "dword" => {
                    if let Err(e) = crate::regspec::parse_dword(value) {
                        errs.push(format!("write_reg.value 无效: {e}"));
                    }
                }
                "bin" | "binary" => {
                    if let Err(e) = crate::regspec::parse_hex_binary(value) {
                        errs.push(format!("write_reg.bin 值无效: {e}"));
                    }
                }
                "expand" | "expand_sz" => {
                    if value.trim().is_empty() {
                        errs.push("write_reg.expand.value 为空".into());
                    }
                }
                "multi_sz" => {
                    if value.split('|').any(|seg| seg.trim().is_empty()) {
                        errs.push(format!(
                            "write_reg.multi_sz 值含空段（`|` 分隔）: {value}"
                        ));
                    }
                }
                other => errs.push(format!(
                    "write_reg.type \"{other}\" 无效（str / dword / bin / expand / multi_sz）"
                )),
            }
            if let Err(e) = crate::regspec::parse_view(view) {
                errs.push(format!("write_reg.view 无效: {e}"));
            }
        }
        MiInstruction::CreateDir { path } => {
            if path.trim().is_empty() {
                errs.push("create_dir.path 为空".into());
            }
        }
        MiInstruction::DeleteFile { path } => {
            if path.trim().is_empty() {
                errs.push("delete_file.path 为空".into());
            }
        }
        MiInstruction::Rename { from, to } => {
            if from.trim().is_empty() {
                errs.push("rename.from 为空".into());
            }
            if to.trim().is_empty() {
                errs.push("rename.to 为空".into());
            }
            if !from.trim().is_empty() && from.trim().eq_ignore_ascii_case(to.trim()) {
                errs.push(format!("rename.from 与 to 相同: {from}"));
            }
        }
        MiInstruction::CopyFiles { from, to } => {
            if from.trim().is_empty() {
                errs.push("copy_files.from 为空".into());
            }
            if to.trim().is_empty() {
                errs.push("copy_files.to 为空".into());
            }
            // 目标在源内部会自嵌套复制（CopyFiles 不允许，NSIS 同语义）
            if let (Some(f), Some(t)) = (norm_path(from), norm_path(to)) {
                if t.starts_with(&format!("{f}\\")) {
                    errs.push(format!(
                        "copy_files.to 在 from 内部（会无限自复制）: {from} → {to}"
                    ));
                }
            }
        }
        MiInstruction::WriteIni { file, section, key, .. } => {
            if file.trim().is_empty() {
                errs.push("write_ini.file 为空".into());
            }
            if section.trim().is_empty() {
                errs.push("write_ini.section 为空".into());
            }
            if key.trim().is_empty() {
                errs.push("write_ini.key 为空".into());
            }
        }
        MiInstruction::Sleep { .. } => {}
        MiInstruction::DetailPrint { .. } => {}
        MiInstruction::ExecShell { target, .. } => {
            if target.trim().is_empty() {
                errs.push("exec_shell.target 为空".into());
            }
        }
        MiInstruction::RmDir { path, .. } => {
            if path.trim().is_empty() {
                errs.push("rmdir.path 为空".into());
            }
        }
        MiInstruction::IfFileExists { path, .. } => {
            if path.trim().is_empty() {
                errs.push("if_file_exists.path 为空".into());
            }
        }
        MiInstruction::Abort { .. } => {}
    }
    errs
}

/// [`validate`] 的递归内核：`depth` 为当前嵌套层数（0 = 顶层），
/// `label` 为错误位置的分支路径前缀（如 `"then>else"`，顶层为空）。
fn validate_at(code: &[MiInstruction], depth: usize, label: &str) -> Vec<String> {
    let mut errs = Vec::new();
    if depth > MAX_CODE_DEPTH {
        // 防御：超限分支块整体拒绝（正常应被下方分支检查拦住）
        errs.push(format!("if_file_exists 嵌套深度超限（≤ {MAX_CODE_DEPTH}）"));
        return errs;
    }
    for (i, inst) in code.iter().enumerate() {
        let at = if label.is_empty() {
            format!("指令 #{}", i + 1)
        } else {
            format!("指令 #{}（{label} 内）", i + 1)
        };
        for inner in validate_one(inst) {
            errs.push(format!("{at}: {inner}"));
        }
        // 分支块递归（与 midiag 诊断层的独立遍历同语义）
        if let MiInstruction::IfFileExists { then, else_branch, .. } = inst {
            let next_depth = depth + 1;
            if next_depth > MAX_CODE_DEPTH {
                errs.push(format!("{at}: if_file_exists 嵌套深度超限（≤ {MAX_CODE_DEPTH}）"));
            } else {
                let then_label = if label.is_empty() { "then".to_string() } else { format!("{label}>then") };
                let else_label = if label.is_empty() { "else".to_string() } else { format!("{label}>else") };
                errs.extend(validate_at(then, next_depth, &then_label));
                errs.extend(validate_at(else_branch, next_depth, &else_label));
            }
        }
    }
    errs
}

/// 指令日志描述（安装详情页友好文案；`vars` 为 None 时不展开变量）。
pub fn describe(inst: &MiInstruction, vars: Option<&BTreeMap<String, String>>) -> String {
    let ex = |s: &str| match vars {
        Some(v) => expand_vars(s, v),
        None => s.to_string(),
    };
    match inst {
        MiInstruction::Run { cmd, args, .. } => {
            if args.trim().is_empty() {
                ex(cmd)
            } else {
                format!("{} {}", ex(cmd), ex(args))
            }
        }
        MiInstruction::WriteReg { key, name, .. } => {
            format!("{}\\{}", ex(key), ex(name))
        }
        MiInstruction::CreateDir { path } => ex(path),
        MiInstruction::DeleteFile { path } => ex(path),
        MiInstruction::Rename { from, to } => format!("{} → {}", ex(from), ex(to)),
        MiInstruction::CopyFiles { from, to } => format!("{} → {}", ex(from), ex(to)),
        MiInstruction::WriteIni { file, section, key, value } => {
            format!("{} [{}] {}={}", ex(file), ex(section), ex(key), ex(value))
        }
        MiInstruction::Sleep { ms } => format!("{ms}ms"),
        MiInstruction::DetailPrint { text } => ex(text),
        MiInstruction::ExecShell { verb, target, args } => {
            let verb = if verb.trim().is_empty() { "open" } else { verb.trim() };
            if args.trim().is_empty() {
                format!("{verb} {}", ex(target))
            } else {
                format!("{verb} {} {}", ex(target), ex(args))
            }
        }
        MiInstruction::RmDir { path, recursive } => {
            if *recursive {
                format!("递归删除 {}", ex(path))
            } else {
                format!("删除空目录 {}", ex(path))
            }
        }
        MiInstruction::IfFileExists { path, then, else_branch } => {
            format!(
                "如果存在 {} → {} 条 / 否则 {} 条",
                ex(path),
                then.len(),
                else_branch.len()
            )
        }
        MiInstruction::Abort { message } => {
            if message.trim().is_empty() {
                "中止安装".to_string()
            } else {
                format!("中止安装: {}", ex(message))
            }
        }
    }
}

/// 路径归一（静态校验用）：反斜杠→正斜杠、去尾部分隔符、大小写不敏感。
fn norm_path(p: &str) -> Option<String> {
    let t = p.trim().replace('/', "\\");
    let t = t.trim_end_matches('\\').to_ascii_lowercase();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

/// NSIS `WriteINIStr` 的 INI 写入语义（Windows `GetPrivateProfileInt` 家族）：
/// 节名与键名匹配**不区分大小写**；键已存在则原位替换，不存在则插入节内
/// 末尾；节不存在则在文件末尾追加；空文件/新文件直接生成两行。
/// 其余行原样保留（含注释与空行）。
/// 纯函数：输入原文件内容，输出新内容；调用方负责读写磁盘。
pub fn write_ini_str(content: &str, section: &str, key: &str, value: &str) -> String {
    let section = section.trim();
    let key = key.trim();
    let sec_head = format!("[{section}]");
    let sec_cmp = sec_head.to_ascii_lowercase();
    let key_cmp = key.to_ascii_lowercase();

    let lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 3);
    let mut in_sec = false;
    let mut sec_found = false;
    let mut key_written = false;

    for line in &lines {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            // 进入新节：若此前还在目标节里且键没写，先补键再开新节
            if in_sec && !key_written {
                out.push(format!("{key}={value}"));
                key_written = true;
            }
            in_sec = trimmed.eq_ignore_ascii_case(&sec_cmp);
            if in_sec {
                sec_found = true;
            }
            out.push(line.clone());
            continue;
        }
        if in_sec {
            // 目标节内的键行：name=value（不区分大小写替换）
            if let Some((k, _)) = trimmed.split_once('=') {
                if k.trim().eq_ignore_ascii_case(&key_cmp) {
                    out.push(format!("{key}={value}"));
                    key_written = true;
                    continue;
                }
            }
        }
        out.push(line.clone());
    }
    if in_sec && !key_written {
        // 文件在目标节内结束：补键
        out.push(format!("{key}={value}"));
    }
    if !sec_found {
        // 节不存在（或文件为空）：末尾追加节头与键
        if !out.is_empty() {
            let last = out.last().map(|s| s.as_str()).unwrap_or("");
            if !last.trim().is_empty() {
                out.push(String::new());
            }
        }
        out.push(sec_head);
        out.push(format!("{key}={value}"));
    }
    let mut res = out.join("\n");
    if !content.ends_with('\n') || content.is_empty() {
        // 保持“无尾换行”形态；空内容时保证以换行收尾更友好
        if content.is_empty() {
            res.push('\n');
        }
    } else {
        res.push('\n');
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("INSTDIR".into(), "C:\\Apps\\Demo".into()),
            ("SMPROGRAMS".into(), "C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs".into()),
            ("WINDIR".into(), "C:\\Windows".into()),
        ])
    }

    /// 每个已登记 op 都能以最小字段反序列化（KNOWN_OPS 与 MiInstruction
    /// 不漂移的守卫；新增指令必须同步登记）。
    #[test]
    fn test_known_ops_all_parse() {
        let minimal: &[(&str, &str)] = &[
            ("run", r#""cmd":"x""#),
            ("write_reg", r#""key":"HKCU\\A","name":"n","value":"v""#),
            ("create_dir", r#""path":"p""#),
            ("delete_file", r#""path":"p""#),
            ("rename", r#""from":"a","to":"b""#),
            ("copy_files", r#""from":"a","to":"b""#),
            ("write_ini", r#""file":"f","section":"s","key":"k","value":"v""#),
            ("sleep", r#""ms":1"#),
            ("detail_print", r#""text":"t""#),
            ("exec_shell", r#""target":"t""#),
            ("rmdir", r#""path":"p""#),
            ("if_file_exists", r#""path":"p""#),
            ("abort", r#""message":"stop""#),
        ];
        assert_eq!(KNOWN_OPS.len(), minimal.len(), "KNOWN_OPS 与最小样例数不一致");
        for (i, (op, fields)) in minimal.iter().enumerate() {
            assert_eq!(KNOWN_OPS[i], *op, "KNOWN_OPS 第 {i} 项与样例顺序不一致");
            let json = format!(r#"{{"op":"{op}",{fields}}}"#);
            let inst: MiInstruction =
                serde_json::from_str(&json).unwrap_or_else(|e| panic!("{op}: {e}"));
            assert_eq!(describe(&inst, None).is_empty(), false, "{op} describe 非空");
        }
    }

    #[test]
    fn test_expand_vars_basic() {
        let v = vars();
        assert_eq!(expand_vars("$INSTDIR\\bin\\x.dll", &v), "C:\\Apps\\Demo\\bin\\x.dll");
        assert_eq!(
            expand_vars("$WINDIR\\notepad.exe", &v),
            "C:\\Windows\\notepad.exe"
        );
    }

    #[test]
    fn test_expand_vars_unknown_kept() {
        let v = vars();
        // 未定义变量原样保留（保守策略）
        assert_eq!(expand_vars("$NOPE\\a", &v), "$NOPE\\a");
        // 混合：已定义部分展开
        assert_eq!(expand_vars("$INSTDIR\\$NOPE", &v), "C:\\Apps\\Demo\\$NOPE");
    }

    #[test]
    fn test_expand_vars_no_vars() {
        assert_eq!(expand_vars("plain path", &BTreeMap::new()), "plain path");
    }

    #[test]
    fn test_validate_run_and_reg() {
        let code = vec![
            MiInstruction::Run { cmd: "".into(), args: String::new(), working_dir: String::new(), abort_on_error: false },
            MiInstruction::WriteReg {
                key: "Software\\Demo".into(),
                name: "x".into(),
                value: "1".into(),
                r#type: String::new(),
                view: String::new(),
            },
            MiInstruction::WriteReg {
                key: "HKCU\\Software\\Demo".into(),
                name: "n".into(),
                value: "0xZZ".into(),
                r#type: "dword".into(),
                view: String::new(),
            },
            MiInstruction::WriteReg {
                key: "HKLM\\Software\\Demo".into(),
                name: "n".into(),
                value: "42".into(),
                r#type: "dword".into(),
                view: String::new(),
            },
        ];
        let errs = validate(&code);
        assert_eq!(errs.len(), 3, "实际: {errs:?}");
        assert!(errs[0].contains("run.cmd"));
        assert!(errs[1].contains("HKLM\\ 或 HKCU\\"));
        assert!(errs[2].contains("write_reg.value"));
    }

    #[test]
    fn test_validate_dir_and_empty_ok() {
        let code = vec![
            MiInstruction::CreateDir { path: "$INSTDIR\\logs".into() },
            MiInstruction::DeleteFile { path: String::new() },
            MiInstruction::DetailPrint { text: String::new() },
        ];
        let errs = validate(&code);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("delete_file.path"));
    }

    #[test]
    fn test_validate_new_instructions() {
        let code = vec![
            MiInstruction::Rename { from: String::new(), to: "b".into() },
            MiInstruction::Rename { from: "C:\\a".into(), to: "c:\\A".into() },
            MiInstruction::CopyFiles { from: "".into(), to: String::new() },
            MiInstruction::CopyFiles {
                from: "$INSTDIR".into(),
                to: "$INSTDIR\\sub".into(),
            },
            MiInstruction::WriteIni {
                file: "a.ini".into(),
                section: String::new(),
                key: String::new(),
                value: String::new(),
            },
            MiInstruction::Sleep { ms: 0 },
        ];
        let errs = validate(&code);
        // ① rename.from 空；② rename 同名（不区分大小写）；③④ copy 双空各一；
        // ⑤ copy 自嵌套；⑥⑦ write_ini section/key 空；⑧ sleep 合法（无错）
        assert_eq!(errs.len(), 7, "实际: {errs:?}");
        assert!(errs[0].contains("rename.from"));
        assert!(errs[1].contains("rename.from 与 to 相同"));
        assert!(errs[2].contains("copy_files.from"));
        assert!(errs[3].contains("copy_files.to"));
        assert!(errs[4].contains("copy_files.to 在 from 内部"));
        assert!(errs[5].contains("write_ini.section"));
        assert!(errs[6].contains("write_ini.key"));
    }

    #[test]
    fn test_describe_new_instructions() {
        let v = vars();
        let rename = MiInstruction::Rename {
            from: "$INSTDIR\\old.ini".into(),
            to: "$INSTDIR\\new.ini".into(),
        };
        assert_eq!(
            describe(&rename, Some(&v)),
            "C:\\Apps\\Demo\\old.ini → C:\\Apps\\Demo\\new.ini"
        );
        let ini = MiInstruction::WriteIni {
            file: "$INSTDIR\\cfg.ini".into(),
            section: "Main".into(),
            key: "Path".into(),
            value: "$INSTDIR".into(),
        };
        assert_eq!(
            describe(&ini, Some(&v)),
            "C:\\Apps\\Demo\\cfg.ini [Main] Path=C:\\Apps\\Demo"
        );
        let sl = MiInstruction::Sleep { ms: 750 };
        assert_eq!(describe(&sl, None), "750ms");
    }

    #[test]
    fn test_serde_roundtrip_new_instructions() {
        let code = vec![
            MiInstruction::Rename { from: "a.ini".into(), to: "b.ini".into() },
            MiInstruction::CopyFiles { from: "$INSTDIR\\plugins".into(), to: "$INSTDIR\\bak".into() },
            MiInstruction::WriteIni {
                file: "$INSTDIR\\cfg.ini".into(),
                section: "Main".into(),
                key: "Mode".into(),
                value: "fast".into(),
            },
            MiInstruction::Sleep { ms: 500 },
        ];
        let json = serde_json::to_string(&code).unwrap();
        assert!(json.contains(r#""op":"rename""#), "{json}");
        assert!(json.contains(r#""op":"copy_files""#));
        assert!(json.contains(r#""op":"write_ini""#));
        assert!(json.contains(r#""op":"sleep""#));
        let back: Vec<MiInstruction> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, code);
        assert!(validate(&back).is_empty());
    }

    #[test]
    fn test_write_ini_new_section_and_empty_file() {
        // 空文件 → 直接生成节 + 键
        let out = write_ini_str("", "Main", "Path", "C:\\x");
        assert_eq!(out, "[Main]\nPath=C:\\x\n");
        // 既有内容追加新节（前空行分隔）
        let src = "[Old]\na=1\n";
        let out = write_ini_str(src, "Main", "Path", "C:\\x");
        assert_eq!(out, "[Old]\na=1\n\n[Main]\nPath=C:\\x\n");
    }

    #[test]
    fn test_write_ini_existing_section_key() {
        // 节存在、键存在：原位替换（节/键匹配不区分大小写，写入用调用方
        // 键名写法——WritePrivateProfileString 语义），其余键保留
        let src = "[Main]\nMode=slow\nPath=old\n[Other]\nk=v\n";
        let out = write_ini_str(src, "main", "Path", "new");
        assert_eq!(out, "[Main]\nMode=slow\nPath=new\n[Other]\nk=v\n");
    }

    #[test]
    fn test_write_ini_insert_into_section_end() {
        // 节存在、键不存在：插在节内末尾（下个节头前）
        let src = "[Main]\nMode=slow\n[Other]\nk=v\n";
        let out = write_ini_str(src, "Main", "Extra", "1");
        assert_eq!(out, "[Main]\nMode=slow\nExtra=1\n[Other]\nk=v\n");
        // 文件在目标节内结束：文件末尾补键
        let src2 = "[Main]\nMode=slow\n";
        let out2 = write_ini_str(src2, "Main", "Extra", "1");
        assert_eq!(out2, "[Main]\nMode=slow\nExtra=1\n");
    }

    #[test]
    fn test_describe_expands() {
        let v = vars();
        let inst = MiInstruction::Run {
            cmd: "$INSTDIR\\setup.bat".into(),
            args: "/q".into(),
            working_dir: String::new(),
            abort_on_error: false,
        };
        assert_eq!(describe(&inst, Some(&v)), "C:\\Apps\\Demo\\setup.bat /q");
        assert_eq!(describe(&inst, None), "$INSTDIR\\setup.bat /q");
    }

    #[test]
    fn test_serde_roundtrip() {
        // TOML 数组表形态的 JSON 等价往返（serde_json 便于测试）
        let src = r#"[
            {"op":"run","cmd":"a.bat","args":"/q","abort_on_error":true},
            {"op":"write_reg","key":"HKCU\\S\\D","name":"Path","value":"$INSTDIR"},
            {"op":"write_reg","key":"HKLM\\S\\D","name":"n","value":"1","type":"dword"},
            {"op":"create_dir","path":"$INSTDIR\\x"},
            {"op":"delete_file","path":"$INSTDIR\\old.log"},
            {"op":"detail_print","text":"done"}
        ]"#;
        let code: Vec<MiInstruction> = serde_json::from_str(src).unwrap();
        assert_eq!(code.len(), 6);
        assert_eq!(code[0], MiInstruction::Run { cmd: "a.bat".into(), args: "/q".into(), working_dir: String::new(), abort_on_error: true });
        // 默认 type = str
        assert_eq!(code[1], MiInstruction::WriteReg { key: "HKCU\\S\\D".into(), name: "Path".into(), value: "$INSTDIR".into(), r#type: String::new(), view: String::new() });
        // 往返
        let json = serde_json::to_string(&code).unwrap();
        let back: Vec<MiInstruction> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, code);
        // 校验干净
        assert!(validate(&code).is_empty());
    }

    // ---- 步骤 39：注册表全类型 + ExecShell + RMDir ----

    #[test]
    fn test_validate_write_reg_full_types() {
        let ok = vec![
            MiInstruction::WriteReg { key: "HKLM\\S".into(), name: "b".into(), value: "DE AD\nbe".into(), r#type: "bin".into(), view: String::new() },
            MiInstruction::WriteReg { key: "HKLM\\S".into(), name: "e".into(), value: "%PATH%".into(), r#type: "expand".into(), view: String::new() },
            MiInstruction::WriteReg { key: "HKLM\\S".into(), name: "m".into(), value: "a|b|c".into(), r#type: "multi_sz".into(), view: "64".into() },
            MiInstruction::WriteReg { key: "HKLM\\S".into(), name: "z".into(), value: String::new(), r#type: "bin".into(), view: String::new() }, // 空 = 零长 REG_BINARY 合法
        ];
        assert!(validate(&ok).is_empty(), "实际: {:?}", validate(&ok));
        let bad = vec![
            MiInstruction::WriteReg { key: "HKLM\\S".into(), name: "b".into(), value: "ABC".into(), r#type: "bin".into(), view: String::new() },      // 奇数长度
            MiInstruction::WriteReg { key: "HKLM\\S".into(), name: "m".into(), value: "a||c".into(), r#type: "multi_sz".into(), view: String::new() }, // 空段
            MiInstruction::WriteReg { key: "HKLM\\S".into(), name: "e".into(), value: "".into(), r#type: "expand".into(), view: String::new() },       // 空值
            MiInstruction::WriteReg { key: "HKLM\\S".into(), name: "v".into(), value: "1".into(), r#type: "str".into(), view: "31".into() },           // 视图非法
            MiInstruction::WriteReg { key: "HKLM\\S".into(), name: "t".into(), value: "1".into(), r#type: "float".into(), view: String::new() },       // 类型非法
        ];
        let errs = validate(&bad);
        assert_eq!(errs.len(), 5, "实际: {errs:?}");
        assert!(errs[0].contains("write_reg.bin 值无效"));
        assert!(errs[1].contains("multi_sz 值含空段"));
        assert!(errs[2].contains("write_reg.expand.value"));
        assert!(errs[3].contains("write_reg.view 无效"));
        assert!(errs[4].contains("bin / expand / multi_sz"));
    }

    #[test]
    fn test_validate_shell_rmdir_abort() {
        let code = vec![
            MiInstruction::ExecShell { verb: String::new(), target: String::new(), args: String::new() }, // target 空
            MiInstruction::ExecShell { verb: "runas".into(), target: "$INSTDIR\\x.exe".into(), args: String::new() }, // 合法
            MiInstruction::RmDir { path: String::new(), recursive: false }, // path 空
            MiInstruction::RmDir { path: "$INSTDIR\\old".into(), recursive: true }, // 合法
            MiInstruction::Abort { message: String::new() }, // 合法（空 message = 静默中止）
        ];
        let errs = validate(&code);
        assert_eq!(errs.len(), 2, "实际: {errs:?}");
        assert!(errs[0].contains("exec_shell.target"));
        assert!(errs[1].contains("rmdir.path"));
    }

    #[test]
    fn test_serde_roundtrip_step39_ops() {
        let code = vec![
            MiInstruction::ExecShell { verb: "open".into(), target: "https://example.com".into(), args: String::new() },
            MiInstruction::ExecShell { verb: String::new(), target: "$INSTDIR\\readme.txt".into(), args: "/p".into() },
            MiInstruction::RmDir { path: "$INSTDIR\\old".into(), recursive: true },
            MiInstruction::RmDir { path: "$INSTDIR\\empty".into(), recursive: false },
            MiInstruction::WriteReg { key: "HKLM\\S".into(), name: "Blob".into(), value: "CAFE".into(), r#type: "bin".into(), view: "32".into() },
        ];
        let json = serde_json::to_string(&code).unwrap();
        assert!(json.contains(r#""op":"exec_shell""#), "{json}");
        assert!(json.contains(r#""op":"rmdir""#));
        assert!(json.contains(r#""view":"32""#));
        // 旧格式兼容：缺省字段（view/args/recursive）serde default 补齐
        let legacy = r#"[
            {"op":"write_reg","key":"HKCU\\S","name":"p","value":"v"}
        ]"#;
        let back: Vec<MiInstruction> = serde_json::from_str(legacy).unwrap();
        assert_eq!(back[0], MiInstruction::WriteReg { key: "HKCU\\S".into(), name: "p".into(), value: "v".into(), r#type: String::new(), view: String::new() });
        let round: Vec<MiInstruction> = serde_json::from_str(&json).unwrap();
        assert_eq!(round, code);
        assert!(validate(&round).is_empty());
    }

    // ---- 步骤 40：if_file_exists + abort ----

    #[test]
    fn test_validate_if_file_exists() {
        // 合法嵌套（then 内 1 条、else 内 1 条）
        let ok = vec![MiInstruction::IfFileExists {
            path: "$INSTDIR\\old.cfg".into(),
            then: vec![MiInstruction::DeleteFile { path: "$INSTDIR\\old.cfg".into() }],
            else_branch: vec![MiInstruction::DetailPrint { text: "无旧配置".into() }],
        }];
        assert!(validate(&ok).is_empty(), "实际: {:?}", validate(&ok));
        // path 空 + then 内错误（错误位置带分支标签）
        let bad = vec![MiInstruction::IfFileExists {
            path: String::new(),
            then: vec![MiInstruction::Run { cmd: String::new(), args: String::new(), working_dir: String::new(), abort_on_error: false }],
            else_branch: vec![],
        }];
        let errs = validate(&bad);
        assert_eq!(errs.len(), 2, "实际: {errs:?}");
        assert!(errs[0].contains("if_file_exists.path"));
        assert!(errs[1].contains("（then 内）"));
    }

    #[test]
    fn test_validate_if_file_exists_depth_limit() {
        // 逐层嵌套至 MAX_CODE_DEPTH+1 层 → 超限报错
        fn nest(level: usize) -> MiInstruction {
            if level == 0 {
                MiInstruction::DetailPrint { text: "leaf".into() }
            } else {
                MiInstruction::IfFileExists {
                    path: "$INSTDIR\\x".into(),
                    then: vec![nest(level - 1)],
                    else_branch: vec![],
                }
            }
        }
        // MAX_CODE_DEPTH 层 if_file_exists：合法（叶子在深度上限内）
        let ok = vec![nest(MAX_CODE_DEPTH)];
        assert!(validate(&ok).is_empty(), "实际: {:?}", validate(&ok));
        // MAX_CODE_DEPTH+1 层 → 超限
        let over = vec![nest(MAX_CODE_DEPTH + 1)];
        let errs = validate(&over);
        assert_eq!(errs.len(), 1, "实际: {errs:?}");
        assert!(errs[0].contains("嵌套深度超限"));
    }

    #[test]
    fn test_serde_roundtrip_if_file_exists() {
        let code = vec![MiInstruction::IfFileExists {
            path: "$INSTDIR\\old".into(),
            then: vec![
                MiInstruction::RmDir { path: "$INSTDIR\\old".into(), recursive: true },
                MiInstruction::IfFileExists {
                    path: "$INSTDIR\\old\\m.ini".into(),
                    then: vec![],
                    else_branch: vec![MiInstruction::Abort { message: "旧配置缺失".into() }],
                },
            ],
            else_branch: vec![MiInstruction::DetailPrint { text: "全新安装".into() }],
        }];
        let json = serde_json::to_string(&code).unwrap();
        assert!(json.contains(r#""op":"if_file_exists""#), "{json}");
        assert!(json.contains(r#""op":"abort""#));
        // else 字段名序列化为 "else"（rename）
        assert!(json.contains(r#""else":"#), "{json}");
        let back: Vec<MiInstruction> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, code);
        assert!(validate(&back).is_empty());
    }

    #[test]
    fn test_describe_step39_40_ops() {
        let v = vars();
        let shell = MiInstruction::ExecShell {
            verb: String::new(),
            target: "https://miedit.example".into(),
            args: String::new(),
        };
        assert_eq!(describe(&shell, Some(&v)), "open https://miedit.example");
        let rmdir = MiInstruction::RmDir { path: "$INSTDIR\\old".into(), recursive: false };
        assert_eq!(describe(&rmdir, Some(&v)), "删除空目录 C:\\Apps\\Demo\\old");
        let rmdirtree = MiInstruction::RmDir { path: "$INSTDIR\\old".into(), recursive: true };
        assert_eq!(describe(&rmdirtree, Some(&v)), "递归删除 C:\\Apps\\Demo\\old");
        let cond = MiInstruction::IfFileExists {
            path: "$INSTDIR\\old.cfg".into(),
            then: vec![MiInstruction::DetailPrint { text: String::new() }],
            else_branch: vec![],
        };
        assert_eq!(describe(&cond, Some(&v)), "如果存在 C:\\Apps\\Demo\\old.cfg → 1 条 / 否则 0 条");
        let abort = MiInstruction::Abort { message: "用户取消".into() };
        assert_eq!(describe(&abort, Some(&v)), "中止安装: 用户取消");
        assert_eq!(describe(&MiInstruction::Abort { message: String::new() }, None), "中止安装");
    }
}
