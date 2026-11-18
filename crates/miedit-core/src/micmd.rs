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
        MiInstruction::DetailPrint { text } => ex(text),
    }
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
