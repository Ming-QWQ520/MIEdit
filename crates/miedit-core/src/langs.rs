//! 多语言框架（NSIS MUI2 `MUI_LANGUAGE` 对齐，路线图步骤 15）。
//!
//! 组成：
//! - 内置语言包：`zh-CN`（完整中文）与 `en-US`（键值回退 [`crate::config::default_text`]
//!   的英文基调），按 `id` / 别名匹配后作为语言文案的"打底"；
//! - 文案解析链在 [`crate::config::resolved_text`]：`ui.text` 单键覆盖（作者强制，
//!   语言无关）> 语言包 `text` 覆盖 > 内置包 > 英文默认；
//! - 系统语言检测：Windows 用 `GetUserDefaultUILanguage`（LANGID → BCP 47 风格 id），
//!   非 Windows（测试/CI）解析 `LC_ALL` / `LANG` 环境变量；
//! - [`initial_language`]：启动时的语言预选（NSIS 语义——定义了多个语言时显示
//!   语言选择页并预选系统语言；只有一个时直接使用）。

use crate::config::Config;

// ---------------------------------------------------------------------------
// 内置语言包
// ---------------------------------------------------------------------------

/// 内置语言包。`texts` 为该语言相对英文默认值的覆盖表。
pub struct BuiltinLanguage {
    /// 语言 id（BCP 47 风格）
    pub id: &'static str,
    /// 别名（如 `zh-Hans`、`zh` 也可命中 zh-CN 包）
    pub aliases: &'static [&'static str],
    /// 文案键值覆盖表
    pub texts: &'static [(&'static str, &'static str)],
}

/// 内置简体中文包（覆盖全部 UI 文案键）。
pub static ZH_CN: BuiltinLanguage = BuiltinLanguage {
    id: "zh-CN",
    aliases: &["zh-Hans", "zh-hans-cn", "zh-sg", "zh"],
    texts: &[
        ("window_title", "{app} 安装向导"),
        ("welcome_chip", "欢 迎"),
        (
            "welcome_body",
            "本向导将把 {app} {version} 安装到你的电脑。\n建议在继续之前关闭其他正在运行的程序。",
        ),
        ("next", "下一步"),
        ("back", "上一步"),
        ("cancel", "取消"),
        ("install_now", "立即安装"),
        ("browse", "浏览..."),
        ("dir_title", "选择安装位置"),
        (
            "dir_desc",
            "{app} 将安装到以下文件夹。\n点击「立即安装」开始，或点击「浏览...」更换位置。",
        ),
        ("license_title", "许可协议"),
        ("license_desc", "请仔细阅读以下许可协议。"),
        ("license_accept", "我接受上述协议条款"),
        ("license_scroll_hint", "（请先将许可协议滚动到底部）"),
        ("installing", "正在安装 {app}"),
        ("preparing", "准备安装..."),
        ("extracting", "正在解压：{file}"),
        ("finalizing", "正在写入卸载程序与快捷方式..."),
        ("finish_title", "安装完成"),
        ("finish_body", "{app} {version} 已成功安装到："),
        ("run_app", "运行 {app}"),
        ("done_btn", "完成"),
        ("error_title", "安装失败"),
        ("exit_body", "安装尚未完成，确定要退出吗？"),
        ("disk_space", "预计需要空间：{size}"),
        ("free_space", "磁盘可用空间：{free}"),
        ("version_line", "版本 {version} · {publisher}"),
        ("components_title", "选择组件"),
        ("components_desc", "请选择要安装的 {app} 组件："),
        ("components_select_all", "全选"),
        ("components_reset", "重置"),
        ("components_required", "必装"),
        ("components_type_title", "安装类型："),
        ("components_type_custom", "自定义"),
        ("components_desc_panel", "组件说明"),
        ("components_desc_empty", "将鼠标悬停在组件上查看说明。"),
        ("app_running_title", "检测到程序正在运行"),
        (
            "app_running_body",
            "{app} 似乎正在运行。请先关闭它后点击「重试检测」，或点击「仍然继续」忽略。",
        ),
        ("retry", "重试检测"),
        ("continue_anyway", "仍然继续"),
        ("upgrade_notice", "检测到已安装版本 {old}，将升级到 {new}。"),
        (
            "file_error_title",
            "写入文件失败",
        ),
        (
            "file_error_body",
            "无法写入以下文件：\n{file}\n\n{error}",
        ),
        (
            "file_error_hint",
            "请关闭正在使用该文件的程序后选择「重试」；选择「忽略」将跳过该文件并在系统重启后替换；选择「中止」将停止安装（已写入的文件保留）。",
        ),
        ("file_error_ignore", "忽略"),
        ("file_error_abort", "中止"),
        (
            "finish_reboot_required",
            "需要重启以完成安装：{count} 个文件将在系统重启后替换。",
        ),
        ("finish_reboot_more", "等共 {count} 个文件（详见 install.log）"),
        ("progress_stats", "已写入 {done} / {total} 个文件 · {bytes} · {speed}"),
        ("details_show", "显示细节"),
        ("details_hide", "隐藏细节"),
        (
            "dir_exists_warning",
            "注意：目标目录已存在且非空，其中的文件将被覆盖或合并。",
        ),
        ("language_title", "选择安装语言"),
        ("language_desc", "请选择安装向导使用的语言。"),
        // 步骤 17：ARP 修复入口 / 修复模式确认页与完成页
        ("repair_title", "修复安装"),
        (
            "repair_body",
            "安装向导已准备修复 {app} {version}。\n将重新解压全部程序文件，并重建快捷方式与注册表项。\n安装目录：{dir}",
        ),
        (
            "repair_summary",
            "文件规则：{files} 条 · 快捷方式：{shortcuts} 个 · 注册表值：{registry} 个",
        ),
        ("repair_now", "立即修复"),
        ("repair_finish_title", "修复完成"),
        ("repair_finish_body", "{app} {version} 已修复完成："),
        // 步骤 20：目录页校验提示（NSIS DirectoryPage：无效路径/根目录拒绝）
        (
            "dir_invalid",
            "安装目录无效。请输入一个已存在盘符上的有效绝对路径（例如 D:\\Apps\\App）。",
        ),
        (
            "dir_root_warning",
            "出于安全考虑，不允许安装到磁盘根目录。请选择一个子文件夹。",
        ),
        // 步骤 21：左侧步骤导航栏（HM NIS Edit 向导式导航）
        ("nav_welcome", "欢迎"),
        ("nav_license", "许可协议"),
        ("nav_components", "选择组件"),
        ("nav_dir", "安装位置"),
        ("nav_install", "正在安装"),
        ("nav_finish", "完成"),
        // 步骤 26：卸载器多语言（NSIS un MUI 对齐）——与英文默认表 un_* 键一一对应；
        // 历史卸载器中文硬编码文案全部移入本表（旧清单无语言包时仍解析为中文，行为不变）
        ("un_window_title", "卸载 {app}"),
        ("un_confirm_title", "卸载 {app}"),
        (
            "un_confirm_body",
            "这将从计算机中移除 {app} {version} 及其所有组件。\n安装目录：{dir}",
        ),
        (
            "un_running_warning",
            "检测到 {app} 可能正在运行，建议关闭后再卸载；\n被占用的文件将在系统重启后删除。",
        ),
        ("un_recheck", "重新检测"),
        ("un_delete_user_data", "同时删除用户数据和配置"),
        ("un_btn_uninstall", "卸载"),
        ("un_btn_next", "下一步"),
        ("un_btn_cancel", "取消"),
        ("un_btn_finish", "完成"),
        ("un_btn_close", "关闭"),
        ("un_working_title", "正在卸载 {app}"),
        ("un_progress_items", "{done} / {total} 项"),
        ("un_summary_title", "已成功卸载 {app}"),
        ("un_stats", "已删除 {files} 个文件、{dirs} 个目录，释放 {size}"),
        ("un_reboot_note", "{count} 个文件将在系统重启后删除。"),
        ("un_done_body", "关闭窗口后，卸载程序将自动清理自身文件。"),
        (
            "un_done_reboot_body",
            "{count} 个文件正在使用中，已计划在系统重启后删除；重启后将完成全部清理。",
        ),
        ("un_failed_title", "卸载遇到问题"),
        ("un_language_title", "选择卸载语言"),
        ("un_language_desc", "请选择卸载向导使用的语言。"),
        ("un_error_admin", "本次安装需要管理员权限才能卸载。"),
        ("un_log_start", "开始卸载 {app}"),
        (
            "un_log_running_warning",
            "警告: 主程序 {exe} 可能正在运行，被占用的文件将在重启后删除",
        ),
        ("un_log_reboot_pending", "注意: {count} 个文件将在系统重启后删除"),
        ("un_log_cmd_label", "卸载前命令 #{n}"),
        ("un_log_exec", "执行{label}: {cmd}"),
        ("un_log_cmd_failed", "{label} 失败: {reason}（已忽略，继续卸载）"),
        ("un_log_reboot_scheduled", "已计划重启后删除（文件占用）: {file}"),
        ("un_log_delete", "删除 {file}"),
        ("un_log_reboot_delete", "重启删除 {file}"),
        ("un_log_remove_dir", "移除目录 {dir}"),
        (
            "un_log_del_file_failed",
            "删除失败: {err}；且无法计划重启删除: {rerr}",
        ),
        ("un_log_del_shortcut_failed", "删除快捷方式失败: {path}"),
        ("un_log_del_shortcut_dir_failed", "删除快捷方式目录失败: {path}"),
        ("un_log_del_user_data", "删除用户数据 {path}"),
        ("un_log_reg_value", "移除注册表值 {key}"),
        ("un_log_reg_key", "移除注册表键 {key}"),
        ("un_log_uninstall_reg", "移除卸载注册表信息"),
        ("un_log_env", "移除环境变量 {name}"),
        ("un_log_env_path", "从 {name} 移除 {segment}"),
        ("un_log_done", "卸载完成"),
        ("un_log_done_errors", "卸载完成，{count} 项未能删除"),
        ("un_silent_stderr", "卸载完成，但有 {count} 个错误（详见日志）"),
    ],
};

/// 内置英文包：键值全部回退 `config::default_text` 的英文基调，无需覆盖表。
pub static EN_US: BuiltinLanguage = BuiltinLanguage {
    id: "en-US",
    aliases: &["en", "en-GB", "en-AU", "en-CA"],
    texts: &[],
};

/// 全部内置语言包。
pub const BUILTINS: &[&BuiltinLanguage] = &[&ZH_CN, &EN_US];

/// 按 id 或别名匹配内置语言包（大小写不敏感）。
pub fn builtin_language_for_id(id: &str) -> Option<&'static BuiltinLanguage> {
    let want = id.trim().to_ascii_lowercase();
    if want.is_empty() {
        return None;
    }
    BUILTINS.iter().copied().find(|b| {
        b.id.eq_ignore_ascii_case(&want)
            || b.aliases.iter().any(|a| a.eq_ignore_ascii_case(&want))
    })
}

// ---------------------------------------------------------------------------
// 语言 id 匹配
// ---------------------------------------------------------------------------

/// 语言 id 匹配（预选系统语言用）：精确相等（大小写不敏感）或主子标签相等
/// （`zh` 与 `zh-CN` / `zh-Hans` 互为匹配；`en` 与 `en-GB` 匹配；跨语言不匹配）。
pub fn lang_matches(def_id: &str, sys_id: &str) -> bool {
    let a = def_id.trim().to_ascii_lowercase();
    let b = sys_id.trim().to_ascii_lowercase();
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a == b {
        return true;
    }
    let pa = a.split('-').next().unwrap_or("");
    let pb = b.split('-').next().unwrap_or("");
    !pa.is_empty() && pa == pb
}

// ---------------------------------------------------------------------------
// 系统语言检测
// ---------------------------------------------------------------------------

/// 常见 Windows LANGID → 语言 id 映射（精确项）。
const LANGIDS: &[(u16, &str)] = &[
    (0x0804, "zh-CN"),
    (0x0404, "zh-TW"),
    (0x0C04, "zh-HK"),
    (0x1004, "zh-SG"),
    (0x0409, "en-US"),
    (0x0809, "en-GB"),
    (0x0C09, "en-AU"),
    (0x1009, "en-CA"),
    (0x0411, "ja-JP"),
    (0x0412, "ko-KR"),
    (0x0407, "de-DE"),
    (0x0807, "de-AT"),
    (0x040C, "fr-FR"),
    (0x0C0C, "fr-CA"),
    (0x0410, "it-IT"),
    (0x040A, "es-ES"),
    (0x080A, "es-MX"),
    (0x0416, "pt-BR"),
    (0x0816, "pt-PT"),
    (0x0419, "ru-RU"),
    (0x0415, "pl-PL"),
    (0x041D, "sv-SE"),
    (0x0414, "nb-NO"),
    (0x0406, "da-DK"),
    (0x0413, "nl-NL"),
    (0x0405, "cs-CZ"),
    (0x0408, "el-GR"),
    (0x040E, "hu-HU"),
    (0x041B, "sk-SK"),
    (0x0424, "sl-SI"),
    (0x041A, "hr-HR"),
    (0x041F, "tr-TR"),
    (0x041E, "th-TH"),
    (0x0421, "id-ID"),
    (0x0425, "et-EE"),
    (0x0426, "lv-LV"),
    (0x0427, "lt-LT"),
    (0x0429, "fa-IR"),
    (0x0402, "bg-BG"),
];

/// 主语言 ID（LANGID 低 10 位）→ 主子标签（无法识别地区时的回退）。
const PRIMARY_LANGS: &[(u16, &str)] = &[
    (0x01, "ar"),
    (0x02, "bg"),
    (0x04, "zh"),
    (0x05, "cs"),
    (0x06, "da"),
    (0x07, "de"),
    (0x08, "el"),
    (0x09, "en"),
    (0x0A, "es"),
    (0x0B, "fi"),
    (0x0C, "fr"),
    (0x0D, "he"),
    (0x0E, "hu"),
    (0x0F, "is"),
    (0x10, "it"),
    (0x11, "ja"),
    (0x12, "ko"),
    (0x13, "nl"),
    (0x14, "no"),
    (0x15, "pl"),
    (0x16, "pt"),
    (0x18, "ro"),
    (0x19, "ru"),
    (0x1A, "hr"),
    (0x1B, "sk"),
    (0x1C, "sq"),
    (0x1D, "sv"),
    (0x1E, "th"),
    (0x1F, "tr"),
    (0x20, "ur"),
    (0x21, "id"),
    (0x22, "uk"),
    (0x24, "sl"),
    (0x25, "et"),
    (0x26, "lv"),
    (0x27, "lt"),
    (0x29, "fa"),
    (0x2A, "vi"),
    (0x2B, "hy"),
    (0x39, "hi"),
];

/// Windows LANGID → BCP 47 风格语言 id。
/// 先查精确表；未命中时回退主语言标签（如 `0x1804` → `zh`）；完全未知返回 None。
pub fn langid_to_id(langid: u16) -> Option<String> {
    if let Some((_, id)) = LANGIDS.iter().find(|(l, _)| *l == langid) {
        return Some((*id).to_string());
    }
    let primary = langid & 0x03FF;
    PRIMARY_LANGS
        .iter()
        .find(|(p, _)| *p == primary)
        .map(|(_, id)| (*id).to_string())
}

/// 解析 Unix 风格语言环境变量值（`LC_ALL` / `LANG`）为 BCP 47 风格 id。
/// `zh_CN.UTF-8` → `zh-CN`；`en` → `en`；`en@euro` → `en`；`C`/`POSIX`/空 → None。
pub fn parse_env_lang(value: &str) -> Option<String> {
    let base = value.split('.').next()?.split('@').next()?.trim();
    if base.is_empty() || base.eq_ignore_ascii_case("C") || base.eq_ignore_ascii_case("POSIX") {
        return None;
    }
    let mut parts = base.splitn(2, ['_', '-']);
    let lang = parts.next()?.trim().to_ascii_lowercase();
    // 主标签必须为 2~8 个字母（BCP 47 语言子标签），过滤 "en_US@x" 之类的畸形输入
    if lang.len() < 2 || lang.len() > 8 || !lang.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    match parts.next() {
        Some(region) if !region.trim().is_empty() => {
            let r: String = region.trim().to_ascii_uppercase();
            Some(format!("{lang}-{r}"))
        }
        _ => Some(lang),
    }
}

/// 当前系统 UI 语言（BCP 47 风格 id）。
/// Windows：`GetUserDefaultUILanguage`；其他平台：`LC_ALL` / `LANG` 环境变量（测试用）。
pub fn detect_system_language() -> Option<String> {
    #[cfg(windows)]
    {
        let langid = crate::winutil::user_default_ui_language();
        langid_to_id(langid)
    }
    #[cfg(not(windows))]
    {
        let v = std::env::var("LC_ALL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| std::env::var("LANG").ok().filter(|s| !s.trim().is_empty()))?;
        parse_env_lang(&v)
    }
}

// ---------------------------------------------------------------------------
// 启动语言预选（NSIS MUI 语义）
// ---------------------------------------------------------------------------

/// 启动时的语言预选索引：
/// - 未定义语言包 → None（沿用英文默认文案，旧包行为不变）；
/// - 单语言 → Some(0)（不显示选择页，直接使用）；
/// - 多语言 → 按系统语言自动匹配（`ui.language_auto_detect` 开启时），否则 Some(0)；
///   匹配顺序：精确 id 匹配优先，其次主标签匹配，最后取第一个。
pub fn initial_language(cfg: &Config) -> Option<usize> {
    let langs = &cfg.ui.languages;
    if langs.is_empty() {
        return None;
    }
    if langs.len() == 1 {
        return Some(0);
    }
    if cfg.ui.language_auto_detect {
        if let Some(sys) = detect_system_language() {
            // 精确匹配优先
            for (i, l) in langs.iter().enumerate() {
                if l.id.trim().eq_ignore_ascii_case(&sys) {
                    return Some(i);
                }
            }
            // 主标签匹配
            for (i, l) in langs.iter().enumerate() {
                if lang_matches(&l.id, &sys) {
                    return Some(i);
                }
            }
        }
    }
    Some(0)
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lang_matches() {
        assert!(lang_matches("zh-CN", "zh-CN"));
        assert!(lang_matches("zh-cn", "ZH-CN")); // 大小写不敏感
        assert!(lang_matches("zh-CN", "zh-Hans")); // 主标签匹配
        assert!(lang_matches("zh", "zh-CN"));
        assert!(lang_matches("en", "en-GB"));
        assert!(!lang_matches("zh", "en-US"));
        assert!(!lang_matches("", "zh"));
        assert!(!lang_matches("zh", ""));
    }

    #[test]
    fn test_langid_to_id_exact() {
        assert_eq!(langid_to_id(0x0804).as_deref(), Some("zh-CN"));
        assert_eq!(langid_to_id(0x0409).as_deref(), Some("en-US"));
        assert_eq!(langid_to_id(0x0411).as_deref(), Some("ja-JP"));
        assert_eq!(langid_to_id(0x0416).as_deref(), Some("pt-BR"));
    }

    #[test]
    fn test_langid_to_id_primary_fallback() {
        // 未知地区变体：主语言为 zh(0x04) → "zh"
        assert_eq!(langid_to_id(0x1804).as_deref(), Some("zh"));
        // 未知地区变体：主语言为 en(0x09) → "en"
        assert_eq!(langid_to_id(0x2809).as_deref(), Some("en"));
        // 主语言未知 → None（调用方回退第一个定义语言）
        assert_eq!(langid_to_id(0x03FF), None);
    }

    #[test]
    fn test_parse_env_lang() {
        assert_eq!(parse_env_lang("zh_CN.UTF-8").as_deref(), Some("zh-CN"));
        assert_eq!(parse_env_lang("en_US").as_deref(), Some("en-US"));
        assert_eq!(parse_env_lang("en").as_deref(), Some("en"));
        assert_eq!(parse_env_lang("en@euro").as_deref(), Some("en"));
        assert_eq!(parse_env_lang("de_DE@euro").as_deref(), Some("de-DE"));
        assert_eq!(parse_env_lang("C"), None);
        assert_eq!(parse_env_lang("POSIX"), None);
        assert_eq!(parse_env_lang(""), None);
        assert_eq!(parse_env_lang("zh.UTF-8").as_deref(), Some("zh"));
    }

    #[test]
    fn test_builtin_language_for_id() {
        assert!(builtin_language_for_id("zh-CN").is_some());
        assert!(builtin_language_for_id("zh-Hans").is_some());
        assert!(builtin_language_for_id("ZH-CN").is_some());
        assert!(builtin_language_for_id("en-US").is_some());
        assert!(builtin_language_for_id("en").is_some());
        // zh 别名命中 zh-CN 包
        assert_eq!(
            builtin_language_for_id("zh").map(|b| b.id),
            Some("zh-CN")
        );
        assert!(builtin_language_for_id("fr-FR").is_none());
        assert!(builtin_language_for_id("").is_none());
    }

    #[test]
    fn test_zh_cn_package_covers_all_default_keys() {
        // 内置中文包必须覆盖英文默认表的全部键（避免双语包键集漂移）
        for (key, _) in crate::config::DEFAULT_TEXT_KEYS {
            assert!(
                ZH_CN.texts.iter().any(|(k, _)| *k == *key),
                "zh-CN 包缺少文案键: {key}"
            );
        }
    }

    #[test]
    fn test_resolved_text_priority_chain() {
        use crate::config::{resolved_text, ui_text, Config, LanguageSpec};
        let mut cfg = Config::default();
        // 1. 无语言包、无覆盖 → 英文默认（旧包行为不变）
        assert_eq!(resolved_text(&cfg, None, "next"), "Next");
        // 2. zh-CN 语言（无覆盖）→ 内置中文包补全
        cfg.ui.languages.push(LanguageSpec {
            id: "zh-CN".into(),
            name: "简体中文".into(),
            text: Default::default(),
        });
        assert_eq!(resolved_text(&cfg, Some(0), "next"), "下一步");
        assert_eq!(resolved_text(&cfg, Some(0), "file_error_abort"), "中止");
        // 3. 语言包 text 覆盖内置包
        cfg.ui.languages[0]
            .text
            .insert("next".into(), "定制下一步".into());
        assert_eq!(resolved_text(&cfg, Some(0), "next"), "定制下一步");
        // 4. ui.text 单键覆盖优先级最高（作者强制，语言无关）
        cfg.ui.text.insert("next".into(), "最终覆盖".into());
        assert_eq!(resolved_text(&cfg, Some(0), "next"), "最终覆盖");
        // 5. 无语言选择（lang=None）→ 跳过语言包，仍走 ui.text/英文默认
        assert_eq!(resolved_text(&cfg, None, "back"), "Back");
        // 6. 越界语言索引安全回退英文默认
        assert_eq!(resolved_text(&cfg, Some(9), "back"), "Back");
        // 7. 未知语言 id → 内置包未命中回退英文默认
        cfg.ui.languages.push(LanguageSpec {
            id: "xx-YY".into(),
            name: "Test".into(),
            text: Default::default(),
        });
        assert_eq!(resolved_text(&cfg, Some(1), "back"), "Back");
        // 8. ui_text 旧入口等价 resolved_text(None)
        assert_eq!(ui_text(&cfg, "back"), "Back");
    }

    #[test]
    fn test_initial_language() {
        let mut cfg = Config::default();
        // 无语言包 → None（旧包行为不变）
        assert_eq!(initial_language(&cfg), None);
        // 单语言 → Some(0)
        cfg.ui.languages.push(crate::config::LanguageSpec {
            id: "zh-CN".into(),
            name: "简体中文".into(),
            text: Default::default(),
        });
        assert_eq!(initial_language(&cfg), Some(0));
        // 多语言 + 关闭自动检测 → Some(0)
        cfg.ui.languages.push(crate::config::LanguageSpec {
            id: "en-US".into(),
            name: "English".into(),
            text: Default::default(),
        });
        cfg.ui.language_auto_detect = false;
        assert_eq!(initial_language(&cfg), Some(0));
    }
}
