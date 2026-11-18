//! 命令行安装参数解析（路线图步骤 21，Linux/CLI 适配）。
//!
//! 安装器命令行（GUI 之外的全自动通道，NSIS `/S` + Inno
//! `/COMPONENTS=` / `/DIR=` 语义合并）：
//!
//! ```text
//! miedit-installer [--silent] [--dir=<路径>] [--components=<a,b,c>]
//!                  [--no-shortcuts] [--ncrc] [--repair] [--help] [--version]
//! ```
//!
//! 退出码：[`EXIT_OK`] 成功；[`EXIT_FAILED`] 安装失败/包损坏；
//! [`EXIT_ARGS`] 参数无效（如 `--components` 引用不存在的组件）。

/// 安装成功。
pub const EXIT_OK: i32 = 0;
/// 安装失败（文件错误/包损坏/系统门槛不足等）。
pub const EXIT_FAILED: i32 = 1;
/// 命令行参数无效。
pub const EXIT_ARGS: i32 = 2;

/// `--components=` 值解析（Inno `/COMPONENTS="a,b"` 语义）：
/// 逗号分隔的组件名（大小写不敏感）或 1 起始序号；
/// 命中项勾选，未提及的组件**不安装**（显式列表即白名单）。
/// 返回 None = 未提供该参数（保持脚本默认勾选）；
/// `Err(())` = 引用了不存在的组件/序号（调用方以退出码 2 退出）。
pub fn parse_components_arg(
    spec: &str,
    component_names: &[String],
) -> Result<Option<Vec<bool>>, ()> {
    let s = spec.trim().trim_matches('"').trim();
    if s.is_empty() {
        return Err(()); // `--components=` 空值视为参数错误
    }
    let mut sel = vec![false; component_names.len()];
    for part in s.split(',').map(|p| p.trim()).filter(|p| !p.is_empty()) {
        // 序号（1 起始）
        let by_index = part
            .parse::<usize>()
            .ok()
            .and_then(|i| i.checked_sub(1))
            .filter(|&i| i < component_names.len());
        if let Some(i) = by_index {
            sel[i] = true;
            continue;
        }
        // 名称匹配（大小写不敏感）
        match component_names
            .iter()
            .position(|n| n.trim().eq_ignore_ascii_case(part))
        {
            Some(i) => sel[i] = true,
            None => return Err(()),
        }
    }
    Ok(Some(sel))
}

/// `--help` 文案（CLI 直出，不进 GUI）。
pub fn help_text(binary: &str, app_name: &str, version: &str) -> String {
    format!(
        "{app_name} {version} 安装程序（MIEdit 构建，NSIS/Inno 对齐）\n\
用法: {binary} [选项]\n\
\n\
选项:\n\
  --silent, /S            静默安装（无向导界面，日志写入 %TEMP%\\miedit-install.log）\n\
  --dir=<路径>, /D=<路径>  指定安装目录（覆盖默认目录）\n\
  --components=<列表>      仅安装列出的组件（逗号分隔的组件名或 1 起始序号）\n\
  --no-shortcuts           不创建快捷方式\n\
  --lang=<id>, /LANG=<id>  指定界面语言（语言 id 如 zh-CN / en，或 NSIS 十进制\n\
                           LCID 如 1033=English、2052=简体中文；无效值回退自动预选）\n\
  --repair, /repair        修复模式（全量重装文件并重建快捷方式/注册表）\n\
  --ncrc, /ncrc            跳过安装包 CRC32 完整性校验\n\
  --help, /?               显示本帮助并退出\n\
  --version, -V            显示版本并退出\n\
\n\
退出码: 0 成功 · 1 安装失败 · 2 参数无效"
    )
}

/// `/LANG=` / `--lang=` 参数提取（NSIS `/LANG=1033` 十进制 LCID 语义，
/// 路线图步骤 35）：返回原始值串（十进制数字或语言 id 如 `zh-CN`），
/// 大小写不敏感前缀匹配；未提供返回 None；值是否有效由
/// [`miedit_core::langs::resolve_lang_override`] 解析（无效值回退自动预选，
/// 不报参数错误——NSIS 同语义）。
pub fn parse_lang_arg(args: &[String]) -> Option<String> {
    for a in args {
        let low = a.to_ascii_lowercase();
        if let Some(rest) = low.strip_prefix("/lang=") {
            return Some(a[a.len() - rest.len()..].trim().to_string());
        }
        if let Some(rest) = low.strip_prefix("--lang=") {
            return Some(a[a.len() - rest.len()..].trim().to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        vec!["Core".into(), "Plugins".into(), "文档 Docs".into()]
    }

    #[test]
    fn test_parse_components_by_name() {
        let sel = parse_components_arg("Core,Plugins", &names()).unwrap().unwrap();
        assert_eq!(sel, vec![true, true, false]);
    }

    #[test]
    fn test_parse_components_case_insensitive_and_spaces() {
        let sel = parse_components_arg(" core , PLUGINS ", &names()).unwrap().unwrap();
        assert_eq!(sel, vec![true, true, false]);
        // 中文组件名
        let sel = parse_components_arg("文档 docs", &names()).unwrap().unwrap();
        assert_eq!(sel, vec![false, false, true]);
    }

    #[test]
    fn test_parse_components_by_index() {
        let sel = parse_components_arg("1,3", &names()).unwrap().unwrap();
        assert_eq!(sel, vec![true, false, true]);
        // 序号越界 → 参数错误
        assert_eq!(parse_components_arg("4", &names()), Err(()));
        assert_eq!(parse_components_arg("0", &names()), Err(()));
    }

    #[test]
    fn test_parse_components_quotes_and_empty() {
        let sel = parse_components_arg("\"Core\"", &names()).unwrap().unwrap();
        assert_eq!(sel, vec![true, false, false]);
        // 空值 = 参数错误
        assert_eq!(parse_components_arg("", &names()), Err(()));
        assert_eq!(parse_components_arg("   ", &names()), Err(()));
        // 未知组件名 = 参数错误
        assert_eq!(parse_components_arg("Nope", &names()), Err(()));
    }

    #[test]
    fn test_help_text_contains_options() {
        let t = help_text("installer.exe", "Demo", "1.0");
        assert!(t.contains("--silent"));
        assert!(t.contains("--components="));
        assert!(t.contains("--no-shortcuts"));
        assert!(t.contains("--lang="));
        assert!(t.contains("Demo 1.0"));
        assert!(t.contains("退出码"));
    }

    // 步骤 35：/LANG= 参数提取（NSIS 十进制 LCID 或语言 id）
    #[test]
    fn test_parse_lang_arg() {
        let a = |v: &[&str]| -> Vec<String> { v.iter().map(|s| s.to_string()).collect() };
        assert_eq!(parse_lang_arg(&a(&["/LANG=1033"])), Some("1033".into()));
        assert_eq!(parse_lang_arg(&a(&["/lang=zh-CN"])), Some("zh-CN".into()));
        assert_eq!(parse_lang_arg(&a(&["--lang= en "])), Some("en".into()));
        assert_eq!(parse_lang_arg(&a(&["/S", "/LANG=2052"])), Some("2052".into()));
        assert_eq!(parse_lang_arg(&a(&["/S", "/D=C:\\x"])), None);
        // 非语言参数（如 /LANGUAGES=xxx）不误匹配
        assert_eq!(parse_lang_arg(&a(&["/language=xx"])), None);
    }
}
