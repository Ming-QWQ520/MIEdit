//! 内置安装界面模板（路线图步骤 21）：五套预设调色板，
//! `ui.template = "<id>"` 一键切换整体视觉；空串 = 不启用模板（沿用
//! 脚本 `ui.accent` 与内置默认色）。模板启用时整体接管调色板
//! （accent/背景渐变/卡片/文字），保证五套模板风格完整一致。

use crate::config::Config;

/// 模板调色板规格（RGB 字节三元组）。
pub struct TemplateSpec {
    /// 模板 id（脚本 `ui.template` 填写，大小写不敏感）
    pub id: &'static str,
    /// 显示名（文档/构建器用）
    pub name: &'static str,
    pub accent: [u8; 3],
    /// 背景渐变顶部/底部（无背景图时的程序化渐变）
    pub bg_top: [u8; 3],
    pub bg_bottom: [u8; 3],
    pub card: [u8; 3],
    pub field: [u8; 3],
    pub card_stroke: [u8; 3],
    pub text: [u8; 3],
    pub text_dim: [u8; 3],
}

/// 五套内置模板。
pub static TEMPLATES: &[TemplateSpec] = &[
    // 1. 极光（默认观感：深蓝夜空 + 蓝紫 accent）
    TemplateSpec {
        id: "aurora",
        name: "极光 Aurora",
        accent: [0x3B, 0x82, 0xF6],
        bg_top: [13, 17, 31],
        bg_bottom: [8, 10, 16],
        card: [10, 12, 18],
        field: [8, 10, 14],
        card_stroke: [52, 56, 66],
        text: [240, 242, 247],
        text_dim: [198, 204, 216],
    },
    // 2. 午夜（深灰蓝 + 青 accent）
    TemplateSpec {
        id: "midnight",
        name: "午夜 Midnight",
        accent: [0x22, 0xD3, 0xEE],
        bg_top: [11, 18, 32],
        bg_bottom: [6, 11, 20],
        card: [11, 20, 32],
        field: [8, 16, 25],
        card_stroke: [42, 69, 96],
        text: [232, 244, 255],
        text_dim: [159, 189, 212],
    },
    // 3. 日出（暖橙粉）
    TemplateSpec {
        id: "sunrise",
        name: "日出 Sunrise",
        accent: [0xF9, 0x73, 0x16],
        bg_top: [31, 18, 6],
        bg_bottom: [18, 10, 5],
        card: [26, 15, 8],
        field: [21, 12, 6],
        card_stroke: [74, 46, 24],
        text: [253, 243, 231],
        text_dim: [217, 184, 153],
    },
    // 4. 森林（深绿）
    TemplateSpec {
        id: "forest",
        name: "森林 Forest",
        accent: [0x34, 0xD3, 0x99],
        bg_top: [10, 23, 16],
        bg_bottom: [6, 16, 10],
        card: [11, 25, 17],
        field: [7, 19, 12],
        card_stroke: [31, 58, 42],
        text: [233, 255, 242],
        text_dim: [166, 203, 180],
    },
    // 5. 单色（极简黑白灰）
    TemplateSpec {
        id: "mono",
        name: "单色 Mono",
        accent: [0xE5, 0xE7, 0xEB],
        bg_top: [16, 16, 16],
        bg_bottom: [10, 10, 10],
        card: [22, 22, 22],
        field: [16, 16, 16],
        card_stroke: [46, 46, 46],
        text: [245, 245, 245],
        text_dim: [176, 176, 176],
    },
];

/// 按 id 查找模板（大小写不敏感；空串返回 None = 不启用模板）。
pub fn template_for_id(id: &str) -> Option<&'static TemplateSpec> {
    let want = id.trim().to_ascii_lowercase();
    if want.is_empty() {
        return None;
    }
    TEMPLATES.iter().find(|t| t.id.eq_ignore_ascii_case(&want))
}

/// 模板全部 id（文档/构建器/校验用）。
pub fn template_ids() -> Vec<&'static str> {
    TEMPLATES.iter().map(|t| t.id).collect()
}

/// 把模板调色板写入配置（`ui.template` 非空时调用）。
/// 返回是否应用了模板；未知 id 忽略（保持脚本原样，宽容处理）。
pub fn apply_template(cfg: &mut Config) -> bool {
    let Some(t) = template_for_id(&cfg.ui.template) else {
        return false;
    };
    let hex = |c: &[u8; 3]| format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]);
    cfg.ui.accent = hex(&t.accent);
    cfg.ui.bg_top = hex(&t.bg_top);
    cfg.ui.bg_bottom = hex(&t.bg_bottom);
    cfg.ui.card = hex(&t.card);
    cfg.ui.field = hex(&t.field);
    cfg.ui.card_stroke = hex(&t.card_stroke);
    cfg.ui.text_color = hex(&t.text);
    cfg.ui.text_dim_color = hex(&t.text_dim);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_template_count_and_ids() {
        // 用户需求：五套安装模板
        assert_eq!(TEMPLATES.len(), 5);
        let ids = template_ids();
        assert_eq!(ids, vec!["aurora", "midnight", "sunrise", "forest", "mono"]);
        // id 唯一
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len());
    }

    #[test]
    fn test_template_for_id_case_insensitive() {
        assert!(template_for_id("aurora").is_some());
        assert!(template_for_id("Aurora").is_some());
        assert!(template_for_id(" MIDNIGHT ").is_some());
        assert!(template_for_id("").is_none());
        assert!(template_for_id("no-such-template").is_none());
    }

    #[test]
    fn test_apply_template_changes_palette() {
        let mut cfg = Config::default();
        let before = cfg.ui.accent.clone();
        cfg.ui.template = "midnight".into();
        assert!(apply_template(&mut cfg));
        assert_eq!(cfg.ui.accent, "#22D3EE");
        assert_eq!(cfg.ui.bg_top, "#0B1220");
        assert_ne!(cfg.ui.accent, before);
        // 再次应用另一模板覆盖生效
        cfg.ui.template = "mono".into();
        assert!(apply_template(&mut cfg));
        assert_eq!(cfg.ui.accent, "#E5E7EB");
    }

    #[test]
    fn test_apply_template_unknown_and_empty_ignored() {
        let mut cfg = Config::default();
        let before = cfg.ui.accent.clone();
        cfg.ui.template = "nope".into();
        assert!(!apply_template(&mut cfg));
        assert_eq!(cfg.ui.accent, before);
        cfg.ui.template = String::new();
        assert!(!apply_template(&mut cfg));
        assert_eq!(cfg.ui.accent, before);
    }

    #[test]
    fn test_all_template_colors_parse_as_hex() {
        // 模板色板必须是合法 #RRGGBB（安装器 parse_hex_color 消费）
        for t in TEMPLATES {
            for (label, c) in [
                ("accent", &t.accent),
                ("bg_top", &t.bg_top),
                ("bg_bottom", &t.bg_bottom),
                ("card", &t.card),
                ("field", &t.field),
                ("card_stroke", &t.card_stroke),
                ("text", &t.text),
                ("text_dim", &t.text_dim),
            ] {
                let _ = label;
                let s = format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]);
                assert!(crate::util::parse_hex_color(&s).is_some(), "{s}");
            }
        }
    }
}
