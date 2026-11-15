//! ARP 修复入口（路线图步骤 17，Inno Setup `ModifyPath` / 修复模式对齐）。
//!
//! 组成：
//! - [`modify_path_value`]：卸载注册表键中的 `ModifyPath` 值——控制面板
//!   「更改/修复」按钮执行该命令，启动安装器进入修复模式；
//! - [`repair_selection`]：修复模式的组件选择——全部勾选（恢复完整安装）；
//! - [`summarize`]：修复摘要统计（确认页展示，纯函数可测）。
//!
//! NSIS 没有内置修复模式（NSIS 写 `NoModify`/`NoRepair` 需手动处理），
//! 本模块语义取自 Inno Setup：默认写 `ModifyPath` 指向安装程序并启用修复。

use crate::config::Config;

/// 修复模式的命令行开关（安装器 main 解析 `--repair` / `/repair`）。
pub const REPAIR_ARG: &str = "--repair";

/// 生成 ARP `ModifyPath` 注册表值。
///
/// 格式：`"<安装器路径>" --repair`（路径带引号，容忍含空格路径）。
/// 安装器路径为空/纯空白时返回 None（无入口，此时回退写 `NoModify`）。
pub fn modify_path_value(installer_path: &str) -> Option<String> {
    let p = installer_path.trim();
    if p.is_empty() {
        return None;
    }
    Some(format!("\"{p}\" {REPAIR_ARG}"))
}

/// 修复模式的组件选择：全量恢复（所有组件勾选）。
///
/// 修复 = 重解压全部文件 + 重建快捷方式/注册表，与 Inno 修复语义一致；
/// 返回长度恰为组件数的全 `true` 向量（无组件配置时为空向量）。
pub fn repair_selection(component_count: usize) -> Vec<bool> {
    vec![true; component_count]
}

/// 修复摘要（确认页展示）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RepairSummary {
    /// 将重装的文件规则条数（`[[files]]` 规则，非实际文件数）
    pub file_rules: usize,
    /// 将重建的快捷方式条目数（含开始菜单/桌面/自定义条目）
    pub shortcuts: usize,
    /// 将重写的自定义注册表值个数（脚本 `[[registry]]` 声明）
    pub registry_values: usize,
}

/// 静态统计修复将重建的内容（纯函数，安装器确认页直接展示）。
///
/// 统计口径与 `finalize` 实际行为一致：
/// - 开始菜单组固定 1 个卸载快捷方式，另有主程序时 +1 个程序快捷方式；
/// - 桌面快捷方式仅在配置开启且有主程序时 1 个；
/// - 自定义条目按其 `locations` 数量计（目标无效时运行期跳过，
///   静态统计按声明数计，允许少量虚高）；
/// - 注册表值只统计键名非空的 `[[registry]]` 规格。
pub fn summarize(cfg: &Config) -> RepairSummary {
    let has_exe = !cfg.app.exe.trim().is_empty();
    let mut shortcuts = 0usize;
    if cfg.shortcuts.start_menu {
        shortcuts += 1; // 卸载 lnk
        if has_exe {
            shortcuts += 1; // 程序 lnk
        }
    }
    if cfg.shortcuts.desktop && has_exe {
        shortcuts += 1;
    }
    for e in &cfg.shortcuts.entries {
        let n = e.locations.iter().filter(|l| !l.trim().is_empty()).count();
        shortcuts += n;
    }
    let registry_values = cfg
        .registry
        .iter()
        .filter(|s| !s.key.trim().is_empty())
        .map(|s| s.values.len())
        .sum();
    RepairSummary {
        file_rules: cfg.files.len(),
        shortcuts,
        registry_values,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{RegistrySpec, ShortcutEntry};

    #[test]
    fn modify_path_quotes_and_flag() {
        let v = modify_path_value(r"C:\Program Files\App\setup.exe").unwrap();
        assert_eq!(v, r#""C:\Program Files\App\setup.exe" --repair"#);
        // 含空格路径整段引号包裹
        let v2 = modify_path_value("C:\\My App\\安装程序.exe").unwrap();
        assert!(v2.starts_with('"') && v2.ends_with(" --repair"));
        assert!(v2.contains("\\My App\\安装程序.exe\" --repair"));
    }

    #[test]
    fn modify_path_trims_and_rejects_empty() {
        assert_eq!(
            modify_path_value("  D:\\x\\s.exe  ").unwrap(),
            "\"D:\\x\\s.exe\" --repair"
        );
        assert!(modify_path_value("").is_none());
        assert!(modify_path_value("   ").is_none());
    }

    #[test]
    fn repair_selection_is_all_true() {
        assert_eq!(repair_selection(0), Vec::<bool>::new());
        assert_eq!(repair_selection(3), vec![true, true, true]);
        // 修复语义：与组件数严格等长（选中状态机可按索引直取）
        let sel = repair_selection(5);
        assert_eq!(sel.len(), 5);
        assert!(sel.iter().all(|b| *b));
    }

    #[test]
    fn summarize_counts_shortcuts_and_registry() {
        let mut cfg = Config::default();
        cfg.app.exe = "app.exe".into();
        cfg.shortcuts.start_menu = true;
        cfg.shortcuts.desktop = true;
        let mut e = ShortcutEntry::default();
        e.name = "Editor".into();
        e.locations = vec!["desktop".into(), "quick_launch".into()];
        cfg.shortcuts.entries.push(e);
        let mut reg = RegistrySpec::default();
        reg.key = r"HKCU\Software\App".into();
        reg.values
            .insert("Path".into(), r"C:\Program Files\App".into());
        reg.values.insert("Version".into(), "1.0".into());
        cfg.registry.push(reg);
        // 空键规格不计
        let mut blank = RegistrySpec::default();
        blank.values.insert("V".into(), "x".into());
        cfg.registry.push(blank);

        let s = summarize(&cfg);
        // start_menu(卸载 lnk + 程序 lnk = 2) + desktop(1) + entry(2) = 5
        assert_eq!(s.shortcuts, 5);
        assert_eq!(s.registry_values, 2);
    }

    #[test]
    fn summarize_no_exe_no_desktop() {
        let mut cfg = Config::default();
        cfg.app.exe = String::new();
        cfg.shortcuts.start_menu = true;
        cfg.shortcuts.desktop = true;
        let s = summarize(&cfg);
        // 无主程序：开始菜单仅卸载 lnk，桌面跳过
        assert_eq!(s.shortcuts, 1);
        assert_eq!(s.file_rules, 0);
        assert_eq!(s.registry_values, 0);
    }
}
