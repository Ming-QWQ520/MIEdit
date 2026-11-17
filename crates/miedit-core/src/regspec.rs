//! 脚本注册表项的类型解析与校验（纯函数，NSIS `WriteRegStr`/`WriteRegDWORD`/
//! `WriteRegBin`/`WriteRegExpandStr`/`WriteRegMultiSz`/`SetRegView` 对齐，
//! 路线图步骤 22）。
//!
//! 数据模型见 [`crate::config::RegistrySpec`]：`values`（REG_SZ）之外，
//! `dwords` / `binary` / `expand_sz` / `multi_sz` 分别携带其余类型，
//! 字符串形式便于 TOML 书写；本模块负责把字符串解析为 [`RegValue`] 并校验格式。

use crate::config::RegistrySpec;
use crate::winutil::{RegValue, RegView};

/// 解析视图字符串（NSIS `SetRegView`）：`default`/`64`/`32`（大小写不敏感；空 = default）。
pub fn parse_view(s: &str) -> Result<RegView, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "" | "default" | "none" => Ok(RegView::Default),
        "64" | "wow64_64" => Ok(RegView::Wow64_64),
        "32" | "wow64_32" => Ok(RegView::Wow64_32),
        other => Err(format!("\"{other}\"：应为 default / 64 / 32")),
    }
}

/// 视图显示名（安装日志用）。
pub fn view_label(view: RegView) -> &'static str {
    match view {
        RegView::Default => "default",
        RegView::Wow64_64 => "64",
        RegView::Wow64_32 => "32",
    }
}

/// DWORD 字符串解析：十进制或 `0x` 前缀十六进制；
/// 负数十进制按 32 位补码回绕（NSIS WriteRegDWORD 传 32 位整数的常见写法）。
pub fn parse_dword(s: &str) -> Result<u32, String> {
    let t = s.trim();
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        if hex.is_empty() {
            return Err("\"0x\"：缺少十六进制数字".into());
        }
        return u32::from_str_radix(hex, 16).map_err(|e| format!("\"{hex}\"：{e}"));
    }
    if t.is_empty() {
        return Err("空字符串".into());
    }
    match t.parse::<i64>() {
        Ok(v) if (0..=u32::MAX as i64).contains(&v) => Ok(v as u32),
        Ok(v) if v < 0 && v >= i32::MIN as i64 => Ok(v as i32 as u32), // 负数按 32 位回绕
        _ => Err(format!("\"{t}\"：超出 32 位范围或格式无效")),
    }
}

/// 十六进制二进制解析：忽略空白（含换行），长度必须为偶数；
/// 空/全空白 = 空字节串（REG_BINARY 允许零长度）。
pub fn parse_hex_binary(s: &str) -> Result<Vec<u8>, String> {
    let compact: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.is_empty() {
        return Ok(Vec::new());
    }
    if compact.len() % 2 != 0 {
        return Err(format!("\"{compact}\"：十六进制长度必须为偶数"));
    }
    (0..compact.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&compact[i..i + 2], 16)
                .map_err(|e| format!("\"{compact}\"：{e}"))
        })
        .collect()
}

/// `{install_dir}` 模板替换。
pub fn expand(s: &str, install_dir: &str) -> String {
    s.replace("{install_dir}", install_dir)
}

/// 聚合一条 `[[registry]]` 的全部值（全部类型 → [`RegValue`]，含 `{install_dir}` 替换）。
/// 任何类型字段解析失败返回 Err（编译期校验与安装期兜底共用）。
pub fn resolve_entries(
    spec: &RegistrySpec,
    install_dir: &str,
) -> Result<Vec<(String, RegValue)>, String> {
    let mut out = Vec::new();
    for (k, v) in &spec.values {
        out.push((k.clone(), RegValue::Str(expand(v, install_dir))));
    }
    for (k, v) in &spec.expand_sz {
        out.push((k.clone(), RegValue::ExpandSz(expand(v, install_dir))));
    }
    for (k, v) in &spec.dwords {
        let d = parse_dword(v).map_err(|e| format!("DWORD 值 {k} 无效: {e}"))?;
        out.push((k.clone(), RegValue::Dword(d)));
    }
    for (k, v) in &spec.binary {
        let b = parse_hex_binary(v).map_err(|e| format!("二进制值 {k} 无效: {e}"))?;
        out.push((k.clone(), RegValue::Binary(b)));
    }
    for (k, v) in &spec.multi_sz {
        out.push((
            k.clone(),
            RegValue::MultiSz(v.iter().map(|s| expand(s, install_dir)).collect()),
        ));
    }
    Ok(out)
}

/// 解析视图并允许 `?` 直接用于 Result 链（便利封装）。
pub fn view_of(spec: &RegistrySpec) -> Result<RegView, String> {
    parse_view(&spec.view)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn spec() -> RegistrySpec {
        RegistrySpec::default()
    }

    #[test]
    fn parse_view_cases() {
        assert_eq!(parse_view(""), Ok(RegView::Default));
        assert_eq!(parse_view("default"), Ok(RegView::Default));
        assert_eq!(parse_view("64"), Ok(RegView::Wow64_64));
        assert_eq!(parse_view(" 32 "), Ok(RegView::Wow64_32));
        assert_eq!(parse_view("WOW64_32"), Ok(RegView::Wow64_32));
        assert!(parse_view("31").is_err());
        assert!(parse_view("abc").is_err());
    }

    #[test]
    fn parse_dword_cases() {
        assert_eq!(parse_dword("1"), Ok(1));
        assert_eq!(parse_dword(" 4294967295 "), Ok(u32::MAX));
        assert_eq!(parse_dword("0x80"), Ok(0x80));
        assert_eq!(parse_dword("0XDEAD"), Ok(0xDEAD));
        // 负数按 32 位补码回绕（NSIS 传 -1 → 0xFFFFFFFF）
        assert_eq!(parse_dword("-1"), Ok(u32::MAX));
        assert_eq!(parse_dword("-2147483648"), Ok(0x8000_0000));
        assert!(parse_dword("").is_err());
        assert!(parse_dword("0x").is_err());
        assert!(parse_dword("99999999999").is_err());
        assert!(parse_dword("abc").is_err());
    }

    #[test]
    fn parse_hex_binary_cases() {
        assert_eq!(parse_hex_binary(""), Ok(vec![]));
        assert_eq!(parse_hex_binary("   "), Ok(vec![]));
        assert_eq!(parse_hex_binary("DEADBEEF"), Ok(vec![0xDE, 0xAD, 0xBE, 0xEF]));
        assert_eq!(
            parse_hex_binary("de ad\nbe\tcd"),
            Ok(vec![0xDE, 0xAD, 0xBE, 0xCD])
        );
        assert!(parse_hex_binary("ABC").is_err()); // 奇数长度
        assert!(parse_hex_binary("ZZ").is_err()); // 非十六进制
    }

    #[test]
    fn expand_placeholder() {
        assert_eq!(
            expand(r"{install_dir}\tools\x.exe", "E:/Demo"),
            r"E:/Demo\tools\x.exe"
        );
        assert_eq!(expand("plain", "E:/Demo"), "plain");
    }

    #[test]
    fn resolve_entries_aggregates_all_types() {
        let mut s = spec();
        s.key = r"HKCU\Software\Demo".into();
        s.values.insert("Path".into(), r"{install_dir}\bin".into());
        s.expand_sz
            .insert("EnvPath".into(), "%PATH%;{install_dir}\\lib".into());
        s.dwords.insert("Mode".into(), "0x10".into());
        s.binary.insert("Blob".into(), "CA FE".into());
        s.multi_sz
            .insert("List".into(), vec!["a".into(), "{install_dir}".into()]);
        let entries = resolve_entries(&s, "D:/App").expect("resolve");
        assert_eq!(entries.len(), 5);
        assert!(entries
            .iter()
            .any(|(n, v)| n == "Path" && matches!(v, RegValue::Str(p) if p == r"D:/App\bin")));
        assert!(entries.iter().any(|(n, v)| n == "Mode"
            && matches!(v, RegValue::Dword(0x10))));
        assert!(entries.iter().any(
            |(n, v)| n == "Blob" && matches!(v, RegValue::Binary(b) if b == &vec![0xCA, 0xFE])
        ));
        assert!(entries.iter().any(|(n, v)| n == "List"
            && matches!(v, RegValue::MultiSz(m) if m == &vec!["a".to_string(), "D:/App".to_string()])));
    }

    #[test]
    fn resolve_entries_reports_bad_field() {
        let mut s = spec();
        s.dwords.insert("Bad".into(), "xyz".into());
        let err = resolve_entries(&s, "").unwrap_err();
        assert!(err.contains("DWORD 值 Bad"), "{err}");
        let mut s2 = spec();
        s2.binary.insert("Bad2".into(), "ABC".into());
        assert!(resolve_entries(&s2, "").unwrap_err().contains("二进制值 Bad2"));
    }
}
