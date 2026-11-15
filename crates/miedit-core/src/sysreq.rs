//! 最小系统版本门槛（Inno `MinVersion` / `OnlyBelowVersion` 风格，路线图步骤 12）。
//!
//! 门槛写法：`"6.1"`（Windows 7）、`"10.0"`、`"10.0.19045"`（可带 build 号）。
//! 比较按 (major, minor, build) 逐段字典序，缺省段补 0；
//! 解析失败视为无效门槛（编译期报错，运行期宽容忽略）。

/// 解析版本门槛字符串为 `[major, minor, build]`。
///
/// 支持前导 `v`、空格；允许 1–3 段数字（`"10"` → `[10,0,0]`）。
/// 非法（空段/负数/超长/非数字）返回 `None`。
pub fn parse_version3(s: &str) -> Option<[u32; 3]> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut out = [0u32; 3];
    let parts: Vec<&str> = s.split('.').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    for (i, p) in parts.iter().enumerate() {
        if p.is_empty() || p.len() > 10 || !p.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        out[i] = p.parse::<u32>().ok()?;
    }
    Some(out)
}

/// 判断当前系统版本是否满足门槛：`current >= minimum` 返回 true。
///
/// `minimum` 无法解析时返回 `Err(原因)`——调用方决定宽容处理或报错；
/// CI/实机验收用高版本门槛验证拒绝路径。
pub fn version_at_least(current: [u32; 3], minimum: &str) -> Result<bool, String> {
    let min = parse_version3(minimum).ok_or_else(|| {
        format!("无效的系统版本门槛 \"{minimum}\"（示例: 6.1 / 10.0 / 10.0.19045）")
    })?;
    Ok(current >= min)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version3_forms() {
        assert_eq!(parse_version3("6.1"), Some([6, 1, 0]));
        assert_eq!(parse_version3("10.0"), Some([10, 0, 0]));
        assert_eq!(parse_version3("10.0.19045"), Some([10, 0, 19045]));
        assert_eq!(parse_version3("10"), Some([10, 0, 0]));
        assert_eq!(parse_version3(" v6.1 "), Some([6, 1, 0]));
        assert_eq!(parse_version3("6.1.0"), Some([6, 1, 0]));
        // 非法输入
        assert_eq!(parse_version3(""), None);
        assert_eq!(parse_version3("   "), None);
        assert_eq!(parse_version3("6.x"), None);
        assert_eq!(parse_version3("6..1"), None);
        assert_eq!(parse_version3("1.2.3.4"), None);
        assert_eq!(parse_version3("-1.0"), None);
    }

    #[test]
    fn at_least_compare_semantics() {
        // Win7 门槛
        assert!(version_at_least([10, 0, 19045], "6.1").unwrap());
        assert!(version_at_least([6, 1, 7601], "6.1").unwrap());
        assert!(!version_at_least([6, 0, 6000], "6.1").unwrap());
        // Win10 门槛：6.1 不满足
        assert!(!version_at_least([6, 1, 7601], "10.0").unwrap());
        // build 段参与比较
        assert!(version_at_least([10, 0, 22000], "10.0.19045").unwrap());
        assert!(!version_at_least([10, 0, 10240], "10.0.19045").unwrap());
        // 无效门槛 → Err（调用方决定策略）
        assert!(version_at_least([10, 0, 0], "abc").is_err());
    }

    #[test]
    fn at_least_err_message_mentions_value() {
        let e = version_at_least([10, 0, 0], "6.1.98.x").unwrap_err();
        assert!(e.contains("6.1.98.x"));
    }
}
