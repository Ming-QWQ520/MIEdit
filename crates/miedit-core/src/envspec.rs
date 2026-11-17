//! 环境变量写入规格解析与 PATH 列表合并（路线图步骤 27）。
//!
//! 对齐对象：
//! - NSIS WriteEnvStr 惯例（写 HKCU/HKLM 环境区 + 广播 WM_SETTINGCHANGE，
//!   PATH 以 `;` 列表去重合并，卸载时从列表中移除本安装写入的段）；
//! - Inno `[Setup] ChangesEnvironment`（安装/卸载后刷新环境）。
//!
//! 本模块为纯函数（拆分/合并/去重/移除/展开），Windows 侧注册表读写与
//! 广播在 [`crate::winutil`]；平台无关逻辑全部可单测。

use crate::config::EnvSpec;

/// 写入方式（`part` 字段解析结果）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvPart {
    /// 直接设值（覆盖现有值）
    All,
    /// 追加到现有 `;` 列表末尾（去重）
    PathAppend,
    /// 前置到现有 `;` 列表头部（去重）
    PathPrepend,
}

/// 作用域（`scope` 字段解析结果）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvScope {
    /// 用户级（HKCU\Environment，无需管理员）
    User,
    /// 机器级（HKLM 环境区，需管理员）
    Machine,
}

impl EnvScope {
    pub fn is_machine(self) -> bool {
        self == EnvScope::Machine
    }
}

/// 解析写入方式（大小写不敏感；无效值返回错误说明）。
pub fn parse_part(s: &str) -> Result<EnvPart, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "" | "all" => Ok(EnvPart::All),
        "path_append" | "append" => Ok(EnvPart::PathAppend),
        "path_prepend" | "prepend" => Ok(EnvPart::PathPrepend),
        other => Err(format!(
            "无效的 part \"{other}\"（可选: all / path_append / path_prepend）"
        )),
    }
}

/// 解析作用域（大小写不敏感；无效值返回错误说明）。
pub fn parse_scope(s: &str) -> Result<EnvScope, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "" | "user" => Ok(EnvScope::User),
        "machine" => Ok(EnvScope::Machine),
        other => Err(format!("无效的 scope \"{other}\"（可选: user / machine）")),
    }
}

/// 展开环境变量值中的模板：`{install_dir}` / `{app}` / `{app_name}`。
pub fn expand_env_value(value: &str, install_dir: &str, app_name: &str) -> String {
    value
        .replace("{install_dir}", install_dir)
        .replace("{app_name}", app_name)
        .replace("{app}", app_name)
}

/// 解析 `;` 分隔的路径/环境列表：按 `;` 拆分、去首尾空白与引号、丢空项。
/// Windows 环境区惯例（PATH 等）使用 `;` 分隔。
pub fn split_path_list(s: &str) -> Vec<String> {
    s.split(';')
        .map(|seg| seg.trim().trim_matches('"').to_string())
        .filter(|seg| !seg.is_empty())
        .collect()
}

/// 拼接 `;` 列表（空列表返回空字符串）。
pub fn join_path_list(items: &[String]) -> String {
    items.join(";")
}

/// 列表合并（NSIS WriteEnvStr PATH 语义）：Windows 环境变量名与路径段
/// 大小写不敏感，故去重按 ASCII 不区分大小写；已存在时先移除旧项再按
/// append/prepend 插入（保证本安装的段位于预期端）；空段不插入。
pub fn merge_path_list(current: &[String], segment: &str, prepend: bool) -> Vec<String> {
    let seg = segment.trim().trim_matches('"');
    if seg.is_empty() {
        return current.to_vec();
    }
    let mut items: Vec<String> = current
        .iter()
        .filter(|it| !it.eq_ignore_ascii_case(seg))
        .cloned()
        .collect();
    if prepend {
        items.insert(0, seg.to_string());
    } else {
        items.push(seg.to_string());
    }
    items
}

/// 从列表中移除一个段（ASCII 不区分大小写）；
/// 返回（移除后的列表, 是否找到）。
pub fn remove_path_segment(current: &[String], segment: &str) -> (Vec<String>, bool) {
    let seg = segment.trim().trim_matches('"');
    if seg.is_empty() {
        return (current.to_vec(), false);
    }
    let before = current.len();
    let items: Vec<String> = current
        .iter()
        .filter(|it| !it.eq_ignore_ascii_case(seg))
        .cloned()
        .collect();
    let found = items.len() != before;
    (items, found)
}

/// 单条环境变量条目的解析结果（安装器写入用）。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedEnvEntry {
    pub name: String,
    pub machine: bool,
    pub part: EnvPart,
    /// 展开后的值（part=all）/ 展开后的单段（path_append/prepend）
    pub value: String,
}

/// 解析并展开一条 EnvSpec；输入非法（part/scope 无法识别、name 为空）返回错误。
pub fn resolve_entry(spec: &EnvSpec, install_dir: &str, app_name: &str) -> Result<ResolvedEnvEntry, String> {
    let name = spec.name.trim().to_string();
    if name.is_empty() {
        return Err("缺少 name".into());
    }
    let part = parse_part(&spec.part)?;
    let scope = parse_scope(&spec.scope)?;
    Ok(ResolvedEnvEntry {
        name,
        machine: scope.is_machine(),
        part,
        value: expand_env_value(spec.value.trim(), install_dir, app_name),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_part_and_scope() {
        assert_eq!(parse_part("all").unwrap(), EnvPart::All);
        assert_eq!(parse_part("").unwrap(), EnvPart::All);
        assert_eq!(parse_part("PATH_APPEND").unwrap(), EnvPart::PathAppend);
        assert_eq!(parse_part(" prepend ").unwrap(), EnvPart::PathPrepend);
        assert!(parse_part("zstd").is_err());
        assert_eq!(parse_scope("").unwrap(), EnvScope::User);
        assert_eq!(parse_scope("Machine").unwrap(), EnvScope::Machine);
        assert!(parse_scope("system").is_err());
    }

    #[test]
    fn test_expand_env_value() {
        assert_eq!(
            expand_env_value("{install_dir}\\bin", "C:\\Apps\\Demo", "Demo"),
            "C:\\Apps\\Demo\\bin"
        );
        assert_eq!(expand_env_value("{app} tools", "C:\\X", "MyApp"), "MyApp tools");
        // {app} 与 {app_name} 同义；{install_dir} 优先不被覆盖
        assert_eq!(expand_env_value("{app_name}/{app}", "", "N"), "N/N");
    }

    #[test]
    fn test_split_join_path_list() {
        let items = split_path_list("C:\\A; C:\\B ;\"C:\\D E\";;");
        assert_eq!(items, vec!["C:\\A", "C:\\B", "C:\\D E"]);
        assert_eq!(join_path_list(&items), "C:\\A;C:\\B;C:\\D E");
        assert!(split_path_list("").is_empty());
        assert!(split_path_list(";;;").is_empty());
    }

    #[test]
    fn test_merge_path_list_dedup_case_insensitive() {
        let cur = vec!["C:\\Windows".to_string(), "C:\\Tools".to_string()];
        // 追加 + 去重（大小写不敏感）
        let merged = merge_path_list(&cur, "c:\\tools", false);
        assert_eq!(merged, vec!["C:\\Windows", "c:\\tools"]);
        // 前置到头部
        let merged = merge_path_list(&cur, "C:\\App", true);
        assert_eq!(merged.first().unwrap(), "C:\\App");
        assert_eq!(merged.len(), 3);
        // 已存在且前置：旧位置移除、新位置在头部
        let merged = merge_path_list(&cur, "C:\\WINDOWS", true);
        assert_eq!(merged.first().unwrap(), "C:\\WINDOWS");
        assert!(!merged[1..].iter().any(|it| it.eq_ignore_ascii_case("C:\\WINDOWS")));
        // 空段 = 不变
        assert_eq!(merge_path_list(&cur, "  ", false), cur);
        // 追加到尾部
        let merged = merge_path_list(&cur, "C:\\End", false);
        assert_eq!(merged.last().unwrap(), "C:\\End");
    }

    #[test]
    fn test_remove_path_segment_roundtrip() {
        let cur = vec!["C:\\A".to_string(), "C:\\B".to_string(), "C:\\C".to_string()];
        let (items, found) = remove_path_segment(&cur, "c:\\b");
        assert!(found);
        assert_eq!(items, vec!["C:\\A", "C:\\C"]);
        let (items2, found2) = remove_path_segment(&items, "C:\\Missing");
        assert!(!found2);
        assert_eq!(items2, items);
        // merge → remove 往返还原
        let merged = merge_path_list(&cur, "C:\\App", true);
        let (back, found3) = remove_path_segment(&merged, "C:\\App");
        assert!(found3);
        assert_eq!(back, cur);
        // 空段不移除任何内容
        let (items4, found4) = remove_path_segment(&cur, "");
        assert!(!found4);
        assert_eq!(items4, cur);
    }

    #[test]
    fn test_resolve_entry() {
        let mut spec = EnvSpec::default();
        spec.name = " JAVA_HOME ".into();
        spec.value = "{install_dir}\\jdk".into();
        spec.part = "path_prepend".into();
        spec.scope = "machine".into();
        let e = resolve_entry(&spec, "C:\\Apps\\Demo", "Demo").unwrap();
        assert_eq!(e.name, "JAVA_HOME");
        assert!(e.machine);
        assert_eq!(e.part, EnvPart::PathPrepend);
        assert_eq!(e.value, "C:\\Apps\\Demo\\jdk");
        // 缺 name 报错
        spec.name = String::new();
        assert!(resolve_entry(&spec, "", "").is_err());
        // 非法 part 报错
        spec.name = "X".into();
        spec.part = "invalid".into();
        assert!(resolve_entry(&spec, "", "").is_err());
    }
}
