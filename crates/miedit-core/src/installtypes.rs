//! 安装类型（NSIS `InstallTypes` / Inno `[Types]` 风格）纯函数逻辑。
//!
//! NSIS 语义对齐：
//! - 组件页顶部列出全部已定义安装类型（Typical/Full/Minimal…），
//!   末尾自动附带一个 **Custom**（自定义）；
//! - 选中某个类型 → 组件勾选状态重置为该类型定义，必装组件始终勾选；
//! - 手动改动任意组件勾选 → 当前选择不再命中任何类型时自动跳到 Custom；
//! - 未定义安装类型时组件页不显示类型行（向后兼容旧脚本/旧包）。

use crate::config::{ComponentSpec, InstallTypeSpec};

/// 计算指定安装类型对应的组件勾选状态。
///
/// 必装组件强制勾选（NSIS 必选 Section 语义）；类型定义中越界的组件索引
/// 被忽略（宽容解析，向后兼容）；`type_index` 越界返回 `None`。
pub fn selection_for_type(
    types: &[InstallTypeSpec],
    components: &[ComponentSpec],
    type_index: usize,
) -> Option<Vec<bool>> {
    let t = types.get(type_index)?;
    Some(
        components
            .iter()
            .enumerate()
            .map(|(i, c)| c.required || t.components.contains(&i))
            .collect(),
    )
}

/// 默认选中的安装类型索引：显式 `default = true` 的优先，否则取第一个；
/// 未定义任何类型时返回 `None`。
pub fn default_type_index(types: &[InstallTypeSpec]) -> Option<usize> {
    types
        .iter()
        .position(|t| t.default)
        .or(if types.is_empty() { None } else { Some(0) })
}

/// 当前组件勾选状态命中的已定义类型索引；全部不命中（= Custom）返回 `None`。
///
/// 比较前按「必装恒为勾选」归一化，与 [`selection_for_type`] 构成互逆状态机：
/// 对任意合法 `i`，`matching_type(t, c, &selection_for_type(t, c, i)?) == Some(i)`。
pub fn matching_type(
    types: &[InstallTypeSpec],
    components: &[ComponentSpec],
    checked: &[bool],
) -> Option<usize> {
    types.iter().position(|t| {
        components
            .iter()
            .enumerate()
            .all(|(i, c)| {
                let want = c.required || t.components.contains(&i);
                checked.get(i).copied().unwrap_or(true) == want
            })
    })
}

/// 组件勾选 → 允许安装的 `[[files]]` 规则索引集合（步骤 21：CLI
/// `--components=` 与 GUI 共用同一口径）。
/// 必装组件始终纳入；未被任何组件引用的规则始终安装（app.rs
/// `selected_rules` 语义）；`files_len` 为文件规则总数。
pub fn selection_to_rules(
    components: &[ComponentSpec],
    files_len: usize,
    checked: &[bool],
) -> std::collections::HashSet<usize> {
    let referenced: std::collections::HashSet<usize> = components
        .iter()
        .flat_map(|c| c.files.iter().copied())
        .collect();
    let mut set = std::collections::HashSet::new();
    for (i, c) in components.iter().enumerate() {
        if c.required || checked.get(i).copied().unwrap_or(true) {
            set.extend(c.files.iter().copied());
        }
    }
    for r in 0..files_len {
        if !referenced.contains(&r) {
            set.insert(r);
        }
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ComponentSpec, InstallTypeSpec};

    #[test]
    fn test_selection_to_rules() {
        // 组件 0=Core(必装,文件0) 1=Plugins(文件1) 2=Docs(文件2)；
        // 文件规则 3 未被任何组件引用 → 始终安装
        let mut core = comp("Core", true);
        core.files = vec![0];
        let mut plugins = comp("Plugins", false);
        plugins.files = vec![1];
        let mut docs = comp("Docs", false);
        docs.files = vec![2];
        let comps = vec![core, plugins, docs];
        let files_len = 4;

        // 只选 Plugins：规则 = {0(必装),1,3(未引用)}
        let sel = vec![false, true, false];
        let rules = selection_to_rules(&comps, files_len, &sel);
        assert!(rules.contains(&0) && rules.contains(&1) && rules.contains(&3));
        assert!(!rules.contains(&2));

        // 全不选（CLI 空白名单）：必装 0 + 未引用 3
        let sel = vec![false, false, false];
        let rules = selection_to_rules(&comps, files_len, &sel);
        assert!(rules.contains(&0) && rules.contains(&3));
        assert!(!rules.contains(&1) && !rules.contains(&2));

        // 越界勾选宽容处理
        let rules = selection_to_rules(&comps, files_len, &[]);
        assert!(rules.contains(&0) && rules.contains(&1) && rules.contains(&2));
    }

    fn comp(name: &str, required: bool) -> ComponentSpec {
        ComponentSpec {
            name: name.into(),
            description: String::new(),
            required,
            files: vec![0],
            group: String::new(),
        }
    }

    fn itype(name: &str, components: &[usize], default: bool) -> InstallTypeSpec {
        InstallTypeSpec {
            name: name.into(),
            components: components.to_vec(),
            default,
        }
    }

    #[test]
    fn selection_basic_and_required_forced() {
        let types = vec![itype("Minimal", &[1], false), itype("Full", &[0, 1, 2], false)];
        let comps = vec![comp("core", true), comp("tools", false), comp("docs", false)];
        let sel = selection_for_type(&types, &comps, 0).unwrap();
        // 组件 0 必装强制勾选；类型只列了 1；2 不勾
        assert_eq!(sel, vec![true, true, false]);
        let full = selection_for_type(&types, &comps, 1).unwrap();
        assert_eq!(full, vec![true, true, true]);
    }

    #[test]
    fn selection_out_of_range_type_returns_none() {
        let types = vec![itype("Full", &[0], false)];
        let comps = vec![comp("core", false)];
        assert!(selection_for_type(&types, &comps, 1).is_none());
        assert!(selection_for_type(&[], &comps, 0).is_none());
    }

    #[test]
    fn selection_ignores_out_of_range_component_index() {
        let types = vec![itype("Full", &[0, 5, 99], false)];
        let comps = vec![comp("a", false), comp("b", false)];
        let sel = selection_for_type(&types, &comps, 0).unwrap();
        assert_eq!(sel, vec![true, false]);
    }

    #[test]
    fn default_type_index_rules() {
        assert!(default_type_index(&[]).is_none());
        let only = vec![itype("Typical", &[0], false)];
        assert_eq!(default_type_index(&only), Some(0));
        let with_default = vec![
            itype("Typical", &[0], false),
            itype("Full", &[0, 1], true),
        ];
        assert_eq!(default_type_index(&with_default), Some(1));
    }

    #[test]
    fn matching_type_exact_and_custom() {
        let types = vec![
            itype("Minimal", &[0], false),
            itype("Full", &[0, 1], false),
        ];
        let comps = vec![comp("core", true), comp("tools", false)];
        // 命中 Full
        assert_eq!(matching_type(&types, &comps, &[true, true]), Some(1));
        // 命中 Minimal（必装恒 true）
        assert_eq!(matching_type(&types, &comps, &[true, false]), Some(0));
        // 手动改勾 → 不命中任何类型 → Custom
        assert_eq!(matching_type(&types, &comps, &[false, false]), None);
        assert_eq!(matching_type(&types, &comps, &[false, true]), None);
        // 未定义类型 → None
        assert_eq!(matching_type(&[], &comps, &[true, false]), None);
    }

    #[test]
    fn matching_is_inverse_of_selection() {
        let types = vec![
            itype("Minimal", &[0], false),
            itype("Full", &[0, 1, 2], false),
            itype("Typical", &[0, 2], true),
        ];
        let comps = vec![comp("core", true), comp("tools", false), comp("docs", false)];
        for i in 0..types.len() {
            let sel = selection_for_type(&types, &comps, i).unwrap();
            assert_eq!(matching_type(&types, &comps, &sel), Some(i), "type #{i} 往返应自洽");
        }
    }

    #[test]
    fn empty_checked_list_defaults_to_all_true_per_slot() {
        // 旧包缺勾选状态时按 true 处理（unwrap_or(true)），必装/非必装均视为勾选
        let types = vec![itype("Full", &[0, 1], false)];
        let comps = vec![comp("a", false), comp("b", false)];
        assert_eq!(matching_type(&types, &comps, &[]), Some(0));
    }
}
