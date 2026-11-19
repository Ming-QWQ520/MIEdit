//! MIEdit 安装包构建向导（GUI）—— HM NIS Edit / Inno Setup 式分步向导。
//!
//! 左侧项目表单（应用/安装/外观/文件/快捷方式/注册表/文案），
//! 右侧实时预览安装界面 + 一键构建。项目与 CLI 脚本（.mis，MI 脚本语言）完全互通。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(deprecated)] // allocate_ui_at_rect 在 0.29 仍稳定可用（0.31 才移除）

mod app;
mod preview;
mod scriptide;

use anyhow::Result;
use eframe::egui::{self, Color32};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let project: Option<std::path::PathBuf> = args.first().map(std::path::PathBuf::from);
    if let Err(e) = real_main(project) {
        eprintln!("{e:#}");
        miedit_core::winutil::message_box_error("MIEdit 构建器", &format!("{e:#}"));
        std::process::exit(1);
    }
}

fn real_main(project: Option<std::path::PathBuf>) -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("MIEdit 安装包构建向导")
            .with_inner_size([1080.0, 760.0])
            .with_min_inner_size([960.0, 680.0]),
        ..Default::default()
    };
    eframe::run_native(
        "MIEdit Builder",
        options,
        Box::new(move |cc| {
            app::install_fonts(&cc.egui_ctx);
            // 初始样式先按深色（BuilderApp::new 读设置后按主题重设一次）
            app::apply_style(&cc.egui_ctx, true);
            Ok(Box::new(app::BuilderApp::new(project, &cc.egui_ctx)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("GUI 运行失败: {e}"))?;
    Ok(())
}

/// 通用颜色（与安装器视觉一致）。
///
/// 深浅色双套（路线图步骤 44：默认跟随系统）。全模块经 [`pal`] 取
/// 当前色，`DARK_MODE` 原子开关由设置页切换（立即生效，无需重启）。
pub struct Palette {
    pub bg_top: Color32,
    pub bg_bottom: Color32,
    pub card: Color32,
    pub card_stroke: Color32,
    pub field: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub btn_bg: Color32,
    pub rail_current: Color32,
    pub faint: Color32,
    pub hover: Color32,
}

/// 深色（默认，与既往视觉一致）。
pub const DARK: Palette = Palette {
    bg_top: Color32::from_rgb(23, 26, 33),
    bg_bottom: Color32::from_rgb(13, 14, 18),
    card: Color32::from_rgb(26, 29, 37),
    card_stroke: Color32::from_rgb(52, 56, 66),
    field: Color32::from_rgb(14, 16, 21),
    text: Color32::from_rgb(238, 240, 245),
    text_dim: Color32::from_rgb(170, 177, 190),
    btn_bg: Color32::from_rgb(32, 36, 46),
    rail_current: Color32::from_rgb(45, 58, 96),
    faint: Color32::from_rgb(31, 34, 43),
    hover: Color32::from_rgb(42, 47, 60),
};

/// 浅色（VSCode Light+ 风格近似）。
pub const LIGHT: Palette = Palette {
    bg_top: Color32::from_rgb(243, 243, 243),
    bg_bottom: Color32::from_rgb(249, 249, 249),
    card: Color32::from_rgb(255, 255, 255),
    card_stroke: Color32::from_rgb(214, 214, 214),
    field: Color32::from_rgb(255, 255, 255),
    text: Color32::from_rgb(31, 31, 31),
    text_dim: Color32::from_rgb(97, 97, 97),
    btn_bg: Color32::from_rgb(228, 228, 228),
    rail_current: Color32::from_rgb(219, 230, 250),
    faint: Color32::from_rgb(240, 240, 240),
    hover: Color32::from_rgb(232, 232, 232),
};

use std::sync::atomic::{AtomicBool, Ordering};

/// 当前是否深色主题（全局原子开关；预览页模拟安装器视觉，不受影响）。
pub static DARK_MODE: AtomicBool = AtomicBool::new(true);

pub fn is_dark() -> bool {
    DARK_MODE.load(Ordering::Relaxed)
}

/// 当前主题调色板。
pub fn pal() -> &'static Palette {
    if is_dark() {
        &DARK
    } else {
        &LIGHT
    }
}
pub const GREEN: Color32 = Color32::from_rgb(0x3D, 0xD6, 0x8C);
pub const RED: Color32 = Color32::from_rgb(0xE5, 0x48, 0x4D);
pub const AMBER: Color32 = Color32::from_rgb(0xF5, 0xA5, 0x24);

pub fn accent_color(cfg: &miedit_core::config::Config) -> Color32 {
    let [r, g, b] = miedit_core::util::parse_hex_color(&cfg.ui.accent).unwrap_or([0x4F, 0x8C, 0xFF]);
    Color32::from_rgb(r, g, b)
}

pub const ACCENT_PRESETS: &[(&str, &str)] = &[
    ("#4F8CFF", "品牌蓝"),
    ("#3B82F6", "默认蓝"),
    ("#7C5CFF", "紫罗兰"),
    ("#E5484D", "珊瑚红"),
    ("#F5A524", "琥珀"),
    ("#3DD68C", "翡翠绿"),
    ("#38BDF8", "天青"),
    ("#F472B6", "樱粉"),
];

pub const WINDOW_HINT: &str = "支持占位符：{autopf} {autopf86} {localappdata} {appdata} {desktop} {docs} {app_name}";

/// 中文文案默认值（新建项目预填，可在“界面文案”中修改）。
pub const TEXT_DEFAULTS_ZH: &[(&str, &str)] = &[
    ("window_title", "{app} 安装向导"),
    ("welcome_chip", "欢 迎"),
    ("welcome_body", "本向导将把 {app} {version} 安装到你的电脑。\n\n建议在继续之前关闭其他正在运行的程序。"),
    ("next", "下一步"),
    ("back", "上一步"),
    ("cancel", "取消"),
    ("install_now", "立即安装"),
    ("browse", "浏览..."),
    ("dir_title", "选择安装位置"),
    ("dir_desc", "{app} 将安装到以下文件夹。\n点击「立即安装」开始，或点击「浏览...」更换位置。"),
    ("license_title", "许可协议"),
    ("license_desc", "请仔细阅读以下许可协议。"),
    ("license_accept", "我接受上述协议条款"),
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
    ("components_title", "选择组件"),
    ("components_desc", "请选择要安装的 {app} 组件："),
    ("components_select_all", "全选"),
    ("components_reset", "重置"),
    ("components_required", "必装"),
    ("free_space", "磁盘可用空间：{free}"),
    ("app_running_title", "检测到程序正在运行"),
    ("app_running_body", "{app} 似乎正在运行。请先关闭它后点击「重试检测」，或点击「仍然继续」忽略。"),
    ("retry", "重试检测"),
    ("continue_anyway", "仍然继续"),
    ("upgrade_notice", "检测到已安装版本 {old}，将升级到 {new}。"),
];

/// 新建项目的默认配置（每用户安装、中文文案）。
pub fn default_config() -> miedit_core::config::Config {
    let mut cfg = miedit_core::config::Config::default();
    cfg.app.name = String::new();
    cfg.app.version = "1.0.0".into();
    cfg.app.publisher = String::new();
    cfg.install.dir_template = "{localappdata}\\Programs\\{app_name}".into();
    cfg.ui.accent = "#4F8CFF".into();
    cfg.ui.window_size = [980.0, 640.0];
    cfg.ui.show_dir_page = true;
    cfg.ui.show_license_page = None;
    cfg.shortcuts.start_menu = true;
    cfg.shortcuts.desktop = true;
    cfg.ui.text = TEXT_DEFAULTS_ZH
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    cfg
}
