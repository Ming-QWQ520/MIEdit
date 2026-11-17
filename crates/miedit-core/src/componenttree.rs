//! 组件分组树（NSIS `SectionGroup`/`SectionGroupEnd` 对齐，路线图步骤 24）。
//!
//! 脚本用 `group = "父组/子组"` 路径字符串表达嵌套（TOML 友好、向后兼容，
//! 等价于 NSIS 的 SectionGroup 嵌套声明）；本模块负责：
//! - 把扁平的 `[[components]]` 构建为带缩进的树行（声明顺序保持、中间组隐式创建）；
//! - 组勾选联动（全选/全不选，必装组件强制保持勾选）；
//! - 组三态（全选/部分/全不选，UI 复选框渲染依据）。
//!
//! 安装类型（InstallTypes）与 CLI `--components=` 仍按扁平组件索引运作，互不影响。

use crate::config::ComponentSpec;

/// 树行类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// 组行（NSIS SectionGroup）
    Group,
    /// 组件行（NSIS Section）
    Component,
}

/// 树行（组件页按序渲染）。
#[derive(Debug, Clone)]
pub struct Row {
    /// 缩进层级（根级 = 0）
    pub depth: usize,
    /// 显示名（组名 / 组件名；空名组件回退「组件 #N」）
    pub label: String,
    pub kind: RowKind,
    /// 组件行对应的 `[[components]]` 索引；组行 = None
    pub component: Option<usize>,
    /// 所在组路径（根级组件为空串；组行 = 自身完整路径）
    pub group_path: String,
}

/// 组路径规范化：去首尾空白与 `/`、按 `/` 分段、丢弃空段、段内去空白后重连。
pub fn normalize_group(group: &str) -> String {
    group
        .split('/')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

/// 构建树行：声明顺序保持；组行在首次出现时插入；中间组隐式创建。
pub fn build(components: &[ComponentSpec]) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    let mut seen_groups: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (idx, c) in components.iter().enumerate() {
        let path = normalize_group(&c.group);
        let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let mut acc = String::new();
        for (i, seg) in segs.iter().enumerate() {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(seg);
            if seen_groups.insert(acc.clone()) {
                rows.push(Row {
                    depth: i,
                    label: (*seg).to_string(),
                    kind: RowKind::Group,
                    component: None,
                    group_path: acc.clone(),
                });
            }
        }
        let label = if c.name.trim().is_empty() {
            format!("组件 #{}", idx + 1)
        } else {
            c.name.trim().to_string()
        };
        rows.push(Row {
            depth: segs.len(),
            label,
            kind: RowKind::Component,
            component: Some(idx),
            group_path: path,
        });
    }
    rows
}

/// 判断组件是否属于某组（路径规范化后精确匹配；空串 = 根级）。
pub fn in_group(group: &str, group_path: &str) -> bool {
    normalize_group(group) == group_path
}

/// 切换组内全部组件勾选：`check = true` 全选、`false` 全不选；
/// 必装组件强制保持勾选（NSIS 必装 Section 语义）。
pub fn toggle_group(
    components: &[ComponentSpec],
    selection: &mut [bool],
    group_path: &str,
    check: bool,
) {
    for (i, c) in components.iter().enumerate() {
        if let Some(slot) = selection.get_mut(i) {
            if in_group(&c.group, group_path) {
                *slot = check || c.required;
            }
        }
    }
}

/// 组勾选三态（NSIS 组件树父组复选框语义）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupState {
    All,
    Partial,
    None_,
}

/// 计算组内勾选状态（空组 = All，便于「默认全选」渲染）。
pub fn group_state(components: &[ComponentSpec], selection: &[bool], group_path: &str) -> GroupState {
    let mut total = 0usize;
    let mut checked = 0usize;
    for (i, c) in components.iter().enumerate() {
        if in_group(&c.group, group_path) {
            total += 1;
            if selection.get(i).copied().unwrap_or(false) {
                checked += 1;
            }
        }
    }
    if total == 0 || checked == total {
        GroupState::All
    } else if checked == 0 {
        GroupState::None_
    } else {
        GroupState::Partial
    }
}

/// 判断某组路径是否位于已折叠组的子树内（UI 跳过渲染）。
pub fn is_hidden(collapsed: &std::collections::BTreeSet<String>, group_path: &str) -> bool {
    let segs: Vec<&str> = group_path.split('/').filter(|s| !s.is_empty()).collect();
    let mut acc = String::new();
    for seg in segs.iter() {
        if !acc.is_empty() {
            acc.push('/');
        }
        acc.push_str(seg);
        if collapsed.contains(&acc) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comp(name: &str, group: &str, required: bool) -> ComponentSpec {
        ComponentSpec {
            name: name.into(),
            description: String::new(),
            required,
            files: vec![0],
            group: group.into(),
        }
    }

    #[test]
    fn build_keeps_order_and_creates_groups() {
        let comps = vec![
            comp("Core", "", false),
            comp("zlib", "Tools/Compression", false),
            comp("gzip", "Tools/Compression", false),
            comp("Editor", "Tools", false),
            comp("Docs", "", false),
        ];
        let rows = build(&comps);
        // 组行首次出现即插入：Tools → Compression → zlib → gzip → Editor(直接在 Tools 下) → Docs
        let kinds: Vec<(RowKind, String, usize)> = rows
            .iter()
            .map(|r| (r.kind, r.label.clone(), r.depth))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (RowKind::Component, "Core".into(), 0),
                (RowKind::Group, "Tools".into(), 0),
                (RowKind::Group, "Compression".into(), 1),
                (RowKind::Component, "zlib".into(), 2),
                (RowKind::Component, "gzip".into(), 2),
                (RowKind::Component, "Editor".into(), 1),
                (RowKind::Component, "Docs".into(), 0),
            ]
        );
        // 组行不重复（第二次进入 Tools/Compression 不再插入组行）
        assert_eq!(rows.iter().filter(|r| r.kind == RowKind::Group).count(), 2);
    }

    #[test]
    fn normalize_group_trims_and_drops_empty() {
        assert_eq!(normalize_group(" / Tools / Compression / "), "Tools/Compression");
        assert_eq!(normalize_group(""), "");
        assert_eq!(normalize_group("///"), "");
    }

    #[test]
    fn toggle_group_respects_required() {
        let comps = vec![
            comp("a", "Tools", true), // 必装
            comp("b", "Tools", false),
            comp("c", "", false),
        ];
        let mut sel = vec![true, true, true];
        toggle_group(&comps, &mut sel, "Tools", false);
        assert_eq!(sel, vec![true, false, true]); // 必装保持勾选
        toggle_group(&comps, &mut sel, "Tools", true);
        assert_eq!(sel, vec![true, true, true]);
        // 根级组不影响 Tools
        toggle_group(&comps, &mut sel, "", false);
        assert_eq!(sel, vec![true, true, false]);
    }

    #[test]
    fn group_state_tri_state() {
        let comps = vec![
            comp("a", "Tools", false),
            comp("b", "Tools", false),
            comp("c", "", false),
        ];
        assert_eq!(
            group_state(&comps, &[true, true, true], "Tools"),
            GroupState::All
        );
        assert_eq!(
            group_state(&comps, &[true, false, true], "Tools"),
            GroupState::Partial
        );
        assert_eq!(
            group_state(&comps, &[false, false, true], "Tools"),
            GroupState::None_
        );
        // 空组 = All（渲染默认勾选态）
        assert_eq!(group_state(&comps, &[], "Nope"), GroupState::All);
    }

    #[test]
    fn is_hidden_checks_ancestors() {
        let mut collapsed = std::collections::BTreeSet::new();
        collapsed.insert("Tools".to_string());
        assert!(is_hidden(&collapsed, "Tools/Compression"));
        assert!(is_hidden(&collapsed, "Tools"));
        assert!(!is_hidden(&collapsed, "Games"));
        assert!(!is_hidden(&collapsed, ""));
        let mut deep = std::collections::BTreeSet::new();
        deep.insert("Tools/Compression".to_string());
        assert!(is_hidden(&deep, "Tools/Compression/Sub"));
        assert!(!is_hidden(&deep, "Tools"));
    }
}
