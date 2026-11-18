//! MI 脚本 v2 指令层（NSIS Section 指令语义，路线图步骤 33）。
//!
//! MI 脚本 v1 是声明式 TOML（文件/注册表/快捷方式等），v2 在每个组件
//! （NSIS `Section`）上叠加**指令式代码段**：组件被勾选后按脚本顺序执行
//! 其 `code` 指令（未勾选的组件指令不执行——NSIS Section 语义）。
//!
//! 指令集（对标 NSIS 核心指令）：
//! | MI 指令        | NSIS 等价          | 说明 |
//! |----------------|--------------------|------|
//! | `run`          | `Exec`/`ExecWait`  | 运行外部命令（复用 [`crate::runcmd`]） |
//! | `write_reg`    | `WriteRegStr`/`WriteRegDWORD` | 写注册表值（str/dword） |
//! | `create_dir`   | `CreateDirectory`  | 创建目录（含父目录） |
//! | `delete_file`  | `Delete`           | 删除文件（缺失静默跳过） |
//! | `rename`       | `Rename`           | 同盘重命名/移动（目标已存在则失败，NSIS 同语义） |
//! | `copy_files`   | `CopyFiles`        | 复制文件或整个目录树到目标 |
//! | `write_ini`    | `WriteINIStr`      | 写 INI 节键值（节/键不区分大小写，Windows API 同语义） |
//! | `sleep`        | `Sleep`            | 暂停指定毫秒 |
//! | `detail_print` | `DetailPrint`      | 详情日志 |
//!
//! 路径与参数支持 NSIS 常量变量（`$INSTDIR`/`$DESKTOP`/`$SMPROGRAMS`…），
//! 安装期由 [`expand_vars`] 展开；[`standard_var_map`] 用真实 Shell 目录
//! 构建变量表。校验为纯函数 [`validate`]，编辑器与安装器共用。

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
    /// NSIS `WriteRegStr`/`WriteRegDWORD`：写单个注册表值。
    /// `type = "str"`（REG_SZ，默认）或 `"dword"`（十进制 / `0x` 十六进制）。
    WriteReg {
        /// 含 hive 前缀的键，如 `HKCU\Software\Demo`
        key: String,
        /// 值名
        name: String,
        /// 值内容（str 支持 NSIS 变量与 `{install_dir}` 模板）
        value: String,
        /// `"str"`（默认）或 `"dword"`
        #[serde(default)]
        r#type: String,
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
}

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
/// 只做**安装期可静态判定**的检查：必填字段、dword 数值合法性、
/// 注册表键 hive 前缀、注册表视图枚举。变量展开后的路径是否存在
/// 属运行期，不在静态校验范围。
pub fn validate(code: &[MiInstruction]) -> Vec<String> {
    let mut errs = Vec::new();
    for (i, inst) in code.iter().enumerate() {
        let at = format!("指令 #{}", i + 1);
        match inst {
            MiInstruction::Run { cmd, .. } => {
                if cmd.trim().is_empty() {
                    errs.push(format!("{at}: run.cmd 为空"));
                }
            }
            MiInstruction::WriteReg { key, name, value, r#type } => {
                let k = key.trim();
                if k.is_empty() {
                    errs.push(format!("{at}: write_reg.key 为空"));
                } else if !k.starts_with("HKLM\\") && !k.starts_with("HKCU\\") {
                    errs.push(format!(
                        "{at}: write_reg.key 需以 HKLM\\ 或 HKCU\\ 开头: {k}"
                    ));
                }
                if name.trim().is_empty() {
                    errs.push(format!("{at}: write_reg.name 为空"));
                }
                let ty = r#type.trim().to_ascii_lowercase();
                match ty.as_str() {
                    "" | "str" => {}
                    "dword" => {
                        if let Err(e) = crate::regspec::parse_dword(value) {
                            errs.push(format!("{at}: write_reg.value 无效: {e}"));
                        }
                    }
                    other => errs.push(format!(
                        "{at}: write_reg.type \"{other}\" 无效（str / dword）"
                    )),
                }
            }
            MiInstruction::CreateDir { path } => {
                if path.trim().is_empty() {
                    errs.push(format!("{at}: create_dir.path 为空"));
                }
            }
            MiInstruction::DeleteFile { path } => {
                if path.trim().is_empty() {
                    errs.push(format!("{at}: delete_file.path 为空"));
                }
            }
            MiInstruction::Rename { from, to } => {
                if from.trim().is_empty() {
                    errs.push(format!("{at}: rename.from 为空"));
                }
                if to.trim().is_empty() {
                    errs.push(format!("{at}: rename.to 为空"));
                }
                if !from.trim().is_empty()
                    && from.trim().eq_ignore_ascii_case(to.trim())
                {
                    errs.push(format!("{at}: rename.from 与 to 相同: {from}"));
                }
            }
            MiInstruction::CopyFiles { from, to } => {
                if from.trim().is_empty() {
                    errs.push(format!("{at}: copy_files.from 为空"));
                }
                if to.trim().is_empty() {
                    errs.push(format!("{at}: copy_files.to 为空"));
                }
                // 目标在源内部会自嵌套复制（CopyFiles 不允许，NSIS 同语义）
                if let (Some(f), Some(t)) = (norm_path(from), norm_path(to)) {
                    if t.starts_with(&format!("{f}\\")) {
                        errs.push(format!(
                            "{at}: copy_files.to 在 from 内部（会无限自复制）: {from} → {to}"
                        ));
                    }
                }
            }
            MiInstruction::WriteIni { file, section, key, .. } => {
                if file.trim().is_empty() {
                    errs.push(format!("{at}: write_ini.file 为空"));
                }
                if section.trim().is_empty() {
                    errs.push(format!("{at}: write_ini.section 为空"));
                }
                if key.trim().is_empty() {
                    errs.push(format!("{at}: write_ini.key 为空"));
                }
            }
            MiInstruction::Sleep { .. } => {}
            MiInstruction::DetailPrint { .. } => {}
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
            },
            MiInstruction::WriteReg {
                key: "HKCU\\Software\\Demo".into(),
                name: "n".into(),
                value: "0xZZ".into(),
                r#type: "dword".into(),
            },
            MiInstruction::WriteReg {
                key: "HKLM\\Software\\Demo".into(),
                name: "n".into(),
                value: "42".into(),
                r#type: "dword".into(),
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
        assert_eq!(code[1], MiInstruction::WriteReg { key: "HKCU\\S\\D".into(), name: "Path".into(), value: "$INSTDIR".into(), r#type: String::new() });
        // 往返
        let json = serde_json::to_string(&code).unwrap();
        let back: Vec<MiInstruction> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, code);
        // 校验干净
        assert!(validate(&code).is_empty());
    }
}
