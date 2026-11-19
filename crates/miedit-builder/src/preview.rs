//! 安装界面实时预览（路线图步骤 41）：从开始至解锁安装的**全流程交互式**
//! 预览界面。
//!
//! - 安装向导：欢迎 → 许可协议（勾选接受才解锁下一步）→ 安装位置 →
//!   选择组件（类型/勾选/描述框联动）→ 正在安装（进度动画 + 解压日志）→ 完成；
//! - 卸载向导：确认卸载 → 正在卸载（进度动画 + 删除日志）→ 卸载完成（统计）；
//! - 等比缩放渲染安装窗口（背景图/主题色/文案/窗口尺寸实时取自当前配置），
//!   按钮真实可点、状态真实可交互（NSIS/HM NIS Edit 向导预览语义）；
//! - 恒定深色视觉（模拟安装器窗口本身，不随构建器主题切换）。

use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, Sense, TextureHandle, Vec2,
};
use crate::accent_color;
use miedit_core::config::{fill_template, ui_text, Config};
use std::time::Instant;

// 安装器窗口视觉（模拟安装器真实观感，恒定深色，不随构建器主题切换）
const CARD: Color32 = Color32::from_rgb(26, 29, 37);
const CARD_STROKE: Color32 = Color32::from_rgb(52, 56, 66);
const FIELD: Color32 = Color32::from_rgb(14, 16, 21);
const TEXT: Color32 = Color32::from_rgb(238, 240, 245);
const TEXT_DIM: Color32 = Color32::from_rgb(170, 177, 190);

/// 预览模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewMode {
    /// 安装向导
    Install,
    /// 卸载向导
    Uninstall,
}

/// 预览会话状态（每次打开重新初始化；内嵌当前配置快照，
/// 脚本编辑未应用到向导也能即时预览）。
pub struct PreviewState {
    pub cfg: Config,
    pub mode: PreviewMode,
    /// 流程内页索引
    pub page: usize,
    /// 安装目录（可编辑，目录页）
    pub install_dir: String,
    /// 许可协议已接受
    pub license_ok: bool,
    /// 组件勾选（与 cfg.components 对齐）
    pub comp_checked: Vec<bool>,
    /// 当前安装类型（usize::MAX = Custom）
    pub type_idx: usize,
    /// 组件描述面板选中项（None = 悬停跟随）
    pub comp_hover: Option<usize>,
    /// 安装进度 0..1
    pub progress: f32,
    /// 安装日志
    pub log: Vec<String>,
    /// 安装是否已完成
    pub install_done: bool,
    /// 完成页「运行程序」勾选
    pub run_after: bool,
    /// 卸载进度 0..1
    pub un_progress: f32,
    /// 卸载日志
    pub un_log: Vec<String>,
    /// 卸载是否完成
    pub un_done: bool,
    /// 卸载完成统计（文件数、字节数）
    pub un_stats: (usize, u64),
    /// 同时删除用户数据（卸载确认页）
    pub un_delete_user_data: bool,
    /// 卸载进度动画起点
    un_started: Option<Instant>,
    /// 卸载日志已推进到的条目数
    un_log_pos: usize,
    /// 安装进度动画起点
    anim_started: Option<Instant>,
}

impl PreviewState {
    /// 新建安装预览（按当前配置初始化：目录模板展开、组件默认勾选、类型默认）。
    pub fn new_install(cfg: &Config) -> Self {
        let dir = miedit_core::util::resolve_path_template(
            &cfg.install.dir_template,
            &cfg.app.name,
        )
        .display()
        .to_string();
        // NSIS 语义：默认全选，required 只锁勾选
        let comp_checked: Vec<bool> = cfg.components.iter().map(|_| true).collect();
        let type_idx = cfg
            .install_types
            .iter()
            .position(|t| t.default)
            .unwrap_or(usize::MAX);
        Self {
            cfg: cfg.clone(),
            mode: PreviewMode::Install,
            page: 0,
            install_dir: dir,
            license_ok: false,
            comp_checked,
            type_idx,
            comp_hover: None,
            progress: 0.0,
            log: Vec::new(),
            install_done: false,
            run_after: true,
            un_progress: 0.0,
            un_log: Vec::new(),
            un_done: false,
            un_stats: (0, 0),
            un_delete_user_data: false,
            un_started: None,
            un_log_pos: 0,
            anim_started: None,
        }
    }

    /// 新建卸载预览（安装目录按当前配置模板展开展示）。
    pub fn new_uninstall(cfg: &Config) -> Self {
        let dir = miedit_core::util::resolve_path_template(
            &cfg.install.dir_template,
            &cfg.app.name,
        )
        .display()
        .to_string();
        Self {
            cfg: cfg.clone(),
            mode: PreviewMode::Uninstall,
            page: 0,
            install_dir: dir,
            license_ok: false,
            comp_checked: Vec::new(),
            type_idx: usize::MAX,
            comp_hover: None,
            progress: 0.0,
            log: Vec::new(),
            install_done: false,
            run_after: false,
            un_progress: 0.0,
            un_log: Vec::new(),
            un_done: false,
            un_stats: (0, 0),
            un_delete_user_data: false,
            un_started: None,
            un_log_pos: 0,
            anim_started: None,
        }
    }
}

/// 安装流程的页面序列（预览侧栏与推进共用）。
fn install_pages(cfg: &Config) -> Vec<&'static str> {
    let mut pages = vec!["welcome"];
    if cfg.license_shown() {
        pages.push("license");
    }
    if cfg.ui.show_dir_page {
        pages.push("dir");
    }
    if cfg.ui.show_components_page != Some(false) && !cfg.components.is_empty() {
        pages.push("components");
    }
    pages.push("install");
    pages.push("finish");
    pages
}

/// 绘制整窗预览（由 app.rs 在 egui::Window 内调用）。
/// 返回 `true` 表示用户点了「完成 / 关闭」，预览可关闭。
pub fn draw_full(
    ui: &mut egui::Ui,
    st: &mut PreviewState,
    tex: Option<&TextureHandle>,
    files_stat: (usize, u64),
    accent: Color32,
) -> bool {
    let cfg = st.cfg.clone();
    let cfg = &cfg; // 借用视图（st 本体仍可变；clone 避免 &mut self 与共享借用冲突）
    let mut close_requested = false;
    let [w, h] = cfg.ui.window_size;
    let s = ui.ctx().pixels_per_point().recip().min(1.0); // HiDPI 兜底
    let size = Vec2::new(w * s, h * s);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter();
    let s_ = |v: f32| v * s;

    // ---- 背景 ----
    match tex {
        Some(t) => {
            let uv = cover_uv(t.size_vec2(), size);
            p.image(t.id(), rect, uv, Color32::WHITE);
            p.rect_filled(rect, 0.0, Color32::from_rgb(10, 12, 20).linear_multiply(0.25));
        }
        None => {
            let top = Color32::from_rgb(13, 17, 31);
            let bottom = Color32::from_rgb(8, 10, 16);
            let n = 24;
            for i in 0..n {
                let t = (i as f32 + 0.5) / n as f32;
                let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
                let c =
                    Color32::from_rgb(f(top.r(), bottom.r()), f(top.g(), bottom.g()), f(top.b(), bottom.b()));
                let y0 = rect.top() + rect.height() * (i as f32 / n as f32);
                let y1 = rect.top() + rect.height() * ((i + 1) as f32 / n as f32);
                p.rect_filled(
                    Rect::from_min_max(Pos2::new(rect.left(), y0), Pos2::new(rect.right(), y1)),
                    0.0,
                    c,
                );
            }
            for i in 0..6 {
                let r = s_(320.0 - i as f32 * 48.0);
                p.circle_filled(
                    Pos2::new(rect.right() - s_(90.0), rect.top() + s_(130.0)),
                    r,
                    accent.linear_multiply(0.10 - i as f32 * 0.014),
                );
            }
        }
    }

    // ---- 标题栏 ----
    let win_title = match st.mode {
        PreviewMode::Install => fill_template(&ui_text(cfg, "window_title"), &cfg.app.name, &cfg.app.version),
        PreviewMode::Uninstall => format!("卸载 {}", cfg.app.name),
    };
    p.text(
        Pos2::new(rect.left() + s_(18.0), rect.top() + s_(22.0)),
        Align2::LEFT_CENTER,
        win_title,
        FontId::proportional(s_(13.5)),
        TEXT_DIM,
    );

    // ---- 卡片 ----
    let card = Rect::from_min_max(
        Pos2::new(rect.left() + s_(44.0), rect.top() + s_(52.0)),
        Pos2::new(rect.right() - s_(44.0), rect.bottom() - s_(104.0)),
    );
    p.rect_filled(card, s_(16.0), CARD.linear_multiply(0.72));
    p.rect(card, s_(16.0), Color32::TRANSPARENT, egui::Stroke::new(1.0_f32, CARD_STROKE));

    // ---- 左侧步骤导航（cfg.ui.show_step_rail）----
    let pages = match st.mode {
        PreviewMode::Install => install_pages(cfg),
        PreviewMode::Uninstall => vec!["un_confirm", "un_working", "un_summary"],
    };
    let nav_labels: Vec<&str> = pages.clone();
    let nav_w = s_(120.0);
    if cfg.ui.show_step_rail || st.mode == PreviewMode::Uninstall {
        for (i, label) in nav_labels.iter().enumerate() {
            let ly = card.top() + s_(26.0) + i as f32 * s_(34.0);
            let cur = i == st.page;
            let bg = if cur { accent.linear_multiply(0.22) } else { Color32::TRANSPARENT };
            p.rect_filled(
                Rect::from_min_size(Pos2::new(card.left() + s_(10.0), ly - s_(9.0)), Vec2::new(nav_w, s_(28.0))),
                s_(7.0),
                bg,
            );
            p.circle_filled(
                Pos2::new(card.left() + s_(26.0), ly + s_(5.0)),
                s_(7.0),
                if cur { accent } else { TEXT_DIM.linear_multiply(0.5) },
            );
            p.text(
                Pos2::new(card.left() + s_(42.0), ly + s_(4.0)),
                Align2::LEFT_CENTER,
                zh_nav_label(label),
                FontId::proportional(s_(12.0)),
                if cur { TEXT } else { TEXT_DIM },
            );
        }
    }
    let content = Rect::from_min_max(
        Pos2::new(
            card.left() + if cfg.ui.show_step_rail || st.mode == PreviewMode::Uninstall { nav_w + s_(16.0) } else { s_(24.0) },
            card.top() + s_(18.0),
        ),
        Pos2::new(card.right() - s_(24.0), card.bottom() - s_(52.0)),
    );

    // ---- 页面内容（egui 真实控件，可交互）----
    let mut btn_next = false;
    let mut btn_back = false;
    let mut btn_close = false;
    egui::Frame::none().show(ui, |ui| {
        ui.allocate_ui_at_rect(content, |ui| {
            ui.style_mut().spacing.item_spacing = Vec2::new(8.0, s_(7.0).max(5.0));
            match st.mode {
                PreviewMode::Install => {
                    draw_install_page(ui, cfg, st, files_stat, accent, content, s, &mut btn_next, &mut btn_back);
                }
                PreviewMode::Uninstall => {
                    draw_uninstall_page(
                        ui, cfg, st, files_stat, accent, content, s, &mut btn_next, &mut btn_close,
                    );
                }
            }
        });
    });

    // ---- 底部导航按钮（安装向导）----
    if st.mode == PreviewMode::Install {
        let by = Rect::from_min_size(
            Pos2::new(rect.right() - s_(44.0) - s_(118.0), rect.bottom() - s_(32.0) - s_(21.0)),
            Vec2::new(s_(118.0), s_(42.0)),
        );
        let next_label = if st.page + 1 == install_pages(cfg).len() - 1 {
            ui_text(cfg, "install_now").to_string()
        } else if last_page_install(cfg, st.page) {
            ui_text(cfg, "done_btn").to_string()
        } else {
            ui_text(cfg, "next").to_string()
        };
        let next_enabled = page_next_enabled(cfg, st, files_stat.0);
        let resp = ui.put(
            by,
            egui::Button::new(egui::RichText::new(next_label).size(s_(15.0).max(11.0)).color(Color32::WHITE))
                .fill(if next_enabled { accent } else { accent.linear_multiply(0.4) })
                .rounding(s_(10.0).max(4.0))
                .min_size(Vec2::new(s_(118.0).max(64.0), s_(42.0).max(24.0))),
        );
        if resp.clicked() && next_enabled {
            if last_page_install(cfg, st.page) {
                btn_close = true;
            } else if st.page + 1 == install_pages(cfg).len() - 1 {
                // 进入安装页并启动动画
                st.page += 1;
                start_install(st, cfg, files_stat);
            } else {
                btn_next = true;
            }
        }
        let back_rect = Rect::from_min_size(
            Pos2::new(by.left() - s_(12.0) - s_(118.0), by.top()),
            by.size(),
        );
        if st.page > 0 && !st.install_done {
            let resp = ui.put(
                back_rect,
                egui::Button::new(egui::RichText::new(ui_text(cfg, "back")).size(s_(15.0).max(11.0)).color(TEXT))
                    .fill(Color32::from_rgb(18, 21, 28))
                    .rounding(s_(10.0).max(4.0))
                    .min_size(by.size()),
            );
            if resp.clicked() {
                btn_back = true;
            }
        }
    }
    let _ = (btn_next, btn_back);
    if btn_next {
        st.page = (st.page + 1).min(pages.len() - 1);
        if st.mode == PreviewMode::Uninstall && st.page == 1 {
            start_uninstall(st, cfg, files_stat);
        }
    }
    if btn_back {
        st.page = st.page.saturating_sub(1);
    }
    // 卸载完成自动推进
    if st.mode == PreviewMode::Uninstall && st.un_done && st.page == 1 {
        st.page = 2;
    }
    // 安装完成自动解锁（进度页 → 完成页需用户点「下一步」→ 已在按钮逻辑中处理）
    if btn_close {
        close_requested = true;
    }
    close_requested
}

/// 是否最后一页（完成页）。
fn last_page_install(cfg: &Config, page: usize) -> bool {
    page + 1 == install_pages(cfg).len()
}

/// 当前页「下一步」是否可用（许可未接受 → 解锁安装的门槛；NSIS license force 语义）。
fn page_next_enabled(cfg: &Config, st: &PreviewState, _files: usize) -> bool {
    let pages = install_pages(cfg);
    match pages.get(st.page).copied() {
        Some("license") => st.license_ok,
        Some("install") => st.install_done,
        _ => true,
    }
}

/// 启动安装动画（日志按规则 + 快捷方式 + 注册表模拟生成）。
fn start_install(st: &mut PreviewState, cfg: &Config, files_stat: (usize, u64)) {
    st.progress = 0.0;
    st.install_done = false;
    st.log.clear();
    let mut logs = vec![format!("准备安装 {} 个文件（{}）", files_stat.0, miedit_core::util::human_size(files_stat.1))];
    for (i, f) in cfg.files.iter().enumerate() {
        let src = if f.source.trim().is_empty() { format!("规则 #{}", i + 1) } else { f.source.clone() };
        logs.push(format!(
            "正在解压：{}",
            std::path::Path::new(&src)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or(src.clone())
        ));
    }
    if cfg.shortcuts.start_menu {
        logs.push("创建开始菜单快捷方式".into());
    }
    if cfg.shortcuts.desktop {
        logs.push("创建桌面快捷方式".into());
    }
    for r in &cfg.registry {
        logs.push(format!("写入注册表 {}", r.key));
    }
    for c in &cfg.install.env {
        logs.push(format!("设置环境变量 {}", c.name));
    }
    logs.push("正在写入卸载程序与快捷方式...".into());
    st.log = logs;
    st.anim_started = Some(Instant::now());
    st.progress = 0.0001;
}

/// 启动卸载动画。
fn start_uninstall(st: &mut PreviewState, cfg: &Config, files_stat: (usize, u64)) {
    st.un_progress = 0.0001;
    st.un_done = false;
    st.un_log_pos = 0;
    st.un_log.clear();
    let mut logs = vec![format!("开始卸载 {}", cfg.app.name)];
    for i in 0..files_stat.0.max(1) {
        logs.push(format!("删除 {}\\bin\\file{:03}.dat", st.install_dir, i));
    }
    if st.un_delete_user_data {
        logs.push(format!("删除用户数据 %APPDATA%\\{}", cfg.app.name));
    }
    logs.push("移除快捷方式".into());
    logs.push("移除注册表卸载信息".into());
    logs.push("移除环境变量 PATH 条目".into());
    logs.push("卸载完成".into());
    st.un_log = logs;
    st.un_stats = (files_stat.0.max(1), files_stat.1);
    st.un_started = Some(Instant::now());
}

/// 卸载页内容。
#[allow(clippy::too_many_arguments)]
fn draw_uninstall_page(
    ui: &mut egui::Ui,
    cfg: &Config,
    st: &mut PreviewState,
    files_stat: (usize, u64),
    accent: Color32,
    content: Rect,
    scale: f32,
    btn_next: &mut bool,
    btn_close: &mut bool,
) {
    let s_ = |v: f32| v * scale;
    let _ = content;
    // 动画推进
    if let Some(t0) = st.un_started {
        if !st.un_done {
            st.un_progress = (t0.elapsed().as_secs_f32() / 2.2).min(1.0);
            let total = st.un_log.len();
            let pos = ((st.un_progress * total as f32).ceil() as usize).min(total);
            st.un_log_pos = pos;
            if st.un_progress >= 1.0 {
                st.un_done = true;
            }
        }
    }
    match st.page {
        0 => {
            ui.add_space(s_(6.0));
            ui.label(egui::RichText::new(format!("卸载 {}", cfg.app.name)).size(s_(22.0).max(14.0)).color(TEXT).strong());
            ui.add_space(s_(8.0));
            ui.label(
                egui::RichText::new(format!(
                    "这将从计算机中移除 {} {} 及其所有组件。\n安装目录：{}",
                    cfg.app.name,
                    cfg.app.version,
                    st.install_dir
                ))
                .size(s_(14.0).max(11.0))
                .color(TEXT_DIM),
            );
            ui.add_space(s_(4.0));
            ui.checkbox(&mut st.un_delete_user_data, egui::RichText::new("同时删除用户数据和配置").size(s_(13.0).max(10.5)).color(TEXT));
            ui.add_space(s_(10.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new("卸载").size(s_(14.0).max(11.0)).color(Color32::WHITE))
                            .fill(accent)
                            .min_size(Vec2::new(s_(100.0).max(60.0), s_(34.0).max(22.0))),
                    )
                    .clicked()
                {
                    *btn_next = true;
                }
            });
        }
        1 => {
            ui.add_space(s_(6.0));
            ui.label(egui::RichText::new(format!("正在卸载 {}", cfg.app.name)).size(s_(20.0).max(14.0)).color(TEXT).strong());
            ui.add_space(s_(8.0));
            let bar = egui::ProgressBar::new(st.un_progress).show_percentage();
            ui.add(bar);
            ui.label(
                egui::RichText::new(format!(
                    "{}/{} 项",
                    st.un_log_pos,
                    st.un_log.len()
                ))
                .size(s_(12.5).max(10.0))
                .color(TEXT_DIM),
            );
            ui.add_space(s_(6.0));
            egui::ScrollArea::vertical().max_height(s_(180.0).max(90.0)).show(ui, |ui| {
                for line in st.un_log.iter().take(st.un_log_pos) {
                    ui.label(egui::RichText::new(line).size(s_(12.0).max(9.5)).color(TEXT_DIM).monospace());
                }
            });
        }
        _ => {
            ui.add_space(s_(6.0));
            ui.label(egui::RichText::new(format!("已成功卸载 {}", cfg.app.name)).size(s_(22.0).max(14.0)).color(TEXT).strong());
            ui.add_space(s_(8.0));
            ui.label(
                egui::RichText::new(format!(
                    "已删除 {} 个文件，释放 {}",
                    st.un_stats.0,
                    miedit_core::util::human_size(st.un_stats.1)
                ))
                .size(s_(14.0).max(11.0))
                .color(TEXT_DIM),
            );
            ui.add_space(s_(10.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new("关闭").size(s_(14.0).max(11.0)).color(Color32::WHITE))
                            .fill(accent)
                            .min_size(Vec2::new(s_(100.0).max(60.0), s_(34.0).max(22.0))),
                    )
                    .clicked()
                {
                    *btn_close = true;
                }
            });
        }
    }
    let _ = files_stat;
}

/// 安装页内容。
#[allow(clippy::too_many_arguments)]
fn draw_install_page(
    ui: &mut egui::Ui,
    cfg: &Config,
    st: &mut PreviewState,
    files_stat: (usize, u64),
    accent: Color32,
    content: Rect,
    scale: f32,
    _btn_next: &mut bool,
    _btn_back: &mut bool,
) {
    let s_ = |v: f32| v * scale;
    let pages = install_pages(cfg);
    let page = pages.get(st.page).copied().unwrap_or("welcome");
    // 安装动画推进
    if page == "install" && !st.install_done {
        // 动画起点记录在 progress 微小值的首帧
        if st.progress <= 0.0002 {
            st.progress = 0.0002;
            st.anim_started = Some(Instant::now());
        }
        if let Some(t0) = st.anim_started {
            let elapsed = t0.elapsed().as_secs_f32();
            st.progress = (elapsed / 2.8).min(1.0);
            if st.progress >= 1.0 {
                st.install_done = true;
            }
        }
    }
    let fill_t = |key: &str| fill_template(&ui_text(cfg, key), &cfg.app.name, &cfg.app.version);
    match page {
        "welcome" => {
            ui.add_space(s_(6.0));
            // chip
            let chip = fill_t("welcome_chip").to_uppercase();
            ui.label(egui::RichText::new(chip).size(s_(11.5).max(9.5)).color(accent).strong());
            ui.add_space(s_(4.0));
            ui.label(
                egui::RichText::new(&cfg.app.name)
                    .size(s_(40.0).max(20.0))
                    .color(TEXT)
                    .strong(),
            );
            ui.label(
                egui::RichText::new(format!(
                    "版本 {} · {}",
                    if cfg.app.version.is_empty() { "1.0.0" } else { &cfg.app.version },
                    if cfg.app.publisher.is_empty() { "—" } else { &cfg.app.publisher }
                ))
                .size(s_(15.0).max(11.0))
                .color(TEXT_DIM),
            );
            ui.add_space(s_(6.0));
            ui.label(egui::RichText::new(fill_t("welcome_body")).size(s_(14.5).max(11.0)).color(TEXT_DIM));
        }
        "license" => {
            ui.add_space(s_(6.0));
            ui.label(egui::RichText::new(fill_t("license_title")).size(s_(22.0).max(14.0)).color(TEXT).strong());
            ui.label(egui::RichText::new(fill_t("license_desc")).size(s_(13.0).max(10.5)).color(TEXT_DIM));
            ui.add_space(s_(4.0));
            let license_text = cfg.ui.license_text.clone().unwrap_or_default();
            egui::ScrollArea::vertical()
                .max_height(content.height() - s_(110.0))
                .id_salt("pv-license")
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(if license_text.is_empty() { "（许可协议文本为空——请在「目录与授权」页设置许可文件）" } else { &license_text })
                        .size(s_(12.5).max(10.0))
                        .color(TEXT_DIM)
                        .monospace());
                });
            ui.checkbox(
                &mut st.license_ok,
                egui::RichText::new(fill_t("license_accept")).size(s_(13.5).max(10.5)).color(TEXT),
            );
        }
        "dir" => {
            ui.add_space(s_(6.0));
            ui.label(egui::RichText::new(fill_t("dir_title")).size(s_(22.0).max(14.0)).color(TEXT).strong());
            ui.label(egui::RichText::new(fill_t("dir_desc")).size(s_(13.0).max(10.5)).color(TEXT_DIM));
            ui.add_space(s_(6.0));
            egui::Frame::none()
                .fill(FIELD)
                .rounding(s_(8.0).max(4.0))
                .inner_margin(egui::Margin::symmetric(10.0, 7.0))
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut st.install_dir)
                            .hint_text("D:\\Apps\\App")
                            .font(egui::TextStyle::Monospace)
                            .desired_width(ui.available_width())
                            .frame(false),
                    );
                });
            ui.add_space(s_(4.0));
            ui.label(
                egui::RichText::new(format!("磁盘可用空间：1.2 TB（模拟）"))
                    .size(s_(12.5).max(10.0))
                    .color(TEXT_DIM),
            );
            ui.label(
                egui::RichText::new(format!(
                    "预计需要空间：{}（{} 项）",
                    miedit_core::util::human_size(files_stat.1),
                    files_stat.0
                ))
                .size(s_(12.5).max(10.0))
                .color(TEXT_DIM),
            );
        }
        "components" => {
            ui.add_space(s_(6.0));
            ui.label(egui::RichText::new(fill_t("components_title")).size(s_(22.0).max(14.0)).color(TEXT).strong());
            ui.label(egui::RichText::new(fill_t("components_desc")).size(s_(13.0).max(10.5)).color(TEXT_DIM));
            ui.add_space(s_(4.0));
            // 类型行（NSIS InstallTypes）
            if !cfg.install_types.is_empty() {
                ui.horizontal(|ui| {
                    for (i, t) in cfg.install_types.iter().enumerate() {
                        if ui
                            .selectable_label(st.type_idx == i, egui::RichText::new(&t.name).size(s_(13.0).max(10.5)).color(TEXT))
                            .clicked()
                        {
                            st.type_idx = i;
                            // 应用类型勾选集
                            for (ci, c) in cfg.components.iter().enumerate() {
                                st.comp_checked[ci] = t.components.contains(&ci) || c.required;
                            }
                        }
                    }
                    if ui
                        .selectable_label(st.type_idx == usize::MAX, egui::RichText::new("Custom").size(s_(13.0).max(10.5)).color(TEXT))
                        .clicked()
                    {
                        st.type_idx = usize::MAX;
                    }
                });
                ui.separator();
            }
            egui::ScrollArea::vertical()
                .max_height(content.height() - s_(140.0))
                .id_salt("pv-components")
                .show(ui, |ui| {
                    for (i, c) in cfg.components.iter().enumerate() {
                        let mut checked = st.comp_checked[i];
                        let label = format!(
                            "{}{}",
                            c.name,
                            if c.required { "（必需）" } else { "" }
                        );
                        let resp = ui.add_enabled(
                            !c.required,
                            egui::Checkbox::new(&mut checked, egui::RichText::new(label).size(s_(13.5).max(10.5)).color(TEXT)),
                        );
                        if c.required {
                            st.comp_checked[i] = true;
                        } else if resp.changed() {
                            st.comp_checked[i] = checked;
                            st.type_idx = usize::MAX; // 手动改动 → Custom（NSIS 语义）
                        }
                        if resp.hovered() || resp.clicked() {
                            st.comp_hover = Some(i);
                        }
                    }
                });
            ui.separator();
            // 描述框（NSIS components.c 描述面板）
            let desc_i = st.comp_hover.unwrap_or(0);
            let desc = cfg
                .components
                .get(desc_i)
                .map(|c| {
                    if c.description.trim().is_empty() {
                        "（该组件暂无描述）".to_string()
                    } else {
                        c.description.clone()
                    }
                })
                .unwrap_or_default();
            ui.label(egui::RichText::new(desc).size(s_(12.5).max(10.0)).color(TEXT_DIM));
        }
        "install" => {
            ui.add_space(s_(6.0));
            ui.label(egui::RichText::new(fill_t("installing")).size(s_(20.0).max(14.0)).color(TEXT).strong());
            ui.add_space(s_(8.0));
            let bar = egui::ProgressBar::new(st.progress).show_percentage();
            ui.add(bar);
            ui.add_space(s_(6.0));
            egui::ScrollArea::vertical()
                .max_height(content.height() - s_(120.0))
                .stick_to_bottom(true)
                .id_salt("pv-install-log")
                .show(ui, |ui| {
                    let total = st.log.len();
                    let shown = ((st.progress * total as f32).ceil() as usize).min(total);
                    for line in st.log.iter().take(shown) {
                        ui.label(egui::RichText::new(line).size(s_(12.0).max(9.5)).color(TEXT_DIM).monospace());
                    }
                });
        }
        _ => {
            // finish
            ui.add_space(s_(6.0));
            ui.label(egui::RichText::new(fill_t("finish_title")).size(s_(24.0).max(15.0)).color(TEXT).strong());
            ui.add_space(s_(8.0));
            ui.label(egui::RichText::new(fill_t("finish_body")).size(s_(14.0).max(11.0)).color(TEXT_DIM));
            ui.label(egui::RichText::new(&st.install_dir).size(s_(14.0).max(11.0)).color(accent).strong());
            if !cfg.install.run_after.trim().is_empty() || !cfg.install.run_tasks.is_empty() {
                ui.add_space(s_(4.0));
                ui.checkbox(
                    &mut st.run_after,
                    egui::RichText::new(fill_t("run_app")).size(s_(13.5).max(10.5)).color(TEXT),
                );
            }
            ui.add_space(s_(6.0));
            ui.label(
                egui::RichText::new("（预览结束——关闭返回编辑器）")
                    .size(s_(12.0).max(9.5))
                    .color(TEXT_DIM),
            );
        }
    }
}

/// 左侧导航中文标签（与安装器内置语言包一致的固定文案）。
fn zh_nav_label(key: &str) -> &'static str {
    match key {
        "nav_license" | "license" => "许可协议",
        "nav_dir" | "dir" => "安装位置",
        "nav_components" | "components" => "选择组件",
        "nav_install" | "install" => "正在安装",
        "nav_finish" | "finish" => "完成",
        "un_confirm_title" | "un_confirm" => "确认卸载",
        "un_working_title" | "un_working" => "正在卸载",
        "un_summary_title" | "un_summary" => "卸载完成",
        _ => "欢迎",
    }
}

fn cover_uv(img: Vec2, win: Vec2) -> Rect {
    if img.x <= 0.0 || img.y <= 0.0 || win.x <= 0.0 || win.y <= 0.0 {
        return Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));
    }
    let img_ar = img.x / img.y;
    let win_ar = win.x / win.y;
    if img_ar > win_ar {
        let f = win_ar / img_ar;
        let x0 = (1.0 - f) * 0.5;
        Rect::from_min_max(Pos2::new(x0, 0.0), Pos2::new(x0 + f, 1.0))
    } else {
        let f = img_ar / win_ar;
        let y0 = (1.0 - f) * 0.5;
        Rect::from_min_max(Pos2::new(0.0, y0), Pos2::new(1.0, y0 + f))
    }
}


/// 安装选项页的静态欢迎页预览（等比缩小渲染，仅供该页小窗展示；
/// 全流程交互预览见 [`draw_full`]）。
pub fn draw_welcome_static(ui: &mut egui::Ui, tex: Option<&TextureHandle>, cfg: &Config, avail_w: f32) {
    let accent = accent_color(cfg);
    let max_h = 460.0;
    let [w, h] = cfg.ui.window_size;
    let s = (avail_w / w).min(max_h / h).clamp(0.18, 1.0);
    let size = Vec2::new(w * s, h * s);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter();
    let s_ = |v: f32| v * s;
    match tex {
        Some(t) => {
            let uv = cover_uv(t.size_vec2(), size);
            p.image(t.id(), rect, uv, Color32::WHITE);
            p.rect_filled(rect, 0.0, Color32::from_rgb(10, 12, 20).linear_multiply(0.25));
        }
        None => {
            let top = Color32::from_rgb(13, 17, 31);
            let bottom = Color32::from_rgb(8, 10, 16);
            let n = 24;
            for i in 0..n {
                let t = (i as f32 + 0.5) / n as f32;
                let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
                let c = Color32::from_rgb(
                    f(top.r(), bottom.r()),
                    f(top.g(), bottom.g()),
                    f(top.b(), bottom.b()),
                );
                let y0 = rect.top() + rect.height() * (i as f32 / n as f32);
                let y1 = rect.top() + rect.height() * ((i + 1) as f32 / n as f32);
                p.rect_filled(
                    Rect::from_min_max(Pos2::new(rect.left(), y0), Pos2::new(rect.right(), y1)),
                    0.0,
                    c,
                );
            }
            for i in 0..6 {
                let r = s_(320.0 - i as f32 * 48.0);
                p.circle_filled(
                    Pos2::new(rect.right() - s_(90.0), rect.top() + s_(130.0)),
                    r,
                    accent.linear_multiply(0.10 - i as f32 * 0.014),
                );
            }
        }
    }
    p.text(
        Pos2::new(rect.left() + s_(18.0), rect.top() + s_(22.0)),
        Align2::LEFT_CENTER,
        fill_template(&ui_text(cfg, "window_title"), &cfg.app.name, &cfg.app.version),
        FontId::proportional(s_(13.5)),
        TEXT_DIM,
    );
    let card = Rect::from_min_max(
        Pos2::new(rect.left() + s_(44.0), rect.top() + s_(108.0)),
        Pos2::new(rect.right() - s_(44.0), rect.bottom() - s_(104.0)),
    );
    p.rect_filled(card, s_(18.0), CARD.linear_multiply(0.62));
    p.rect(card, s_(18.0), Color32::TRANSPARENT, egui::Stroke::new(1.0_f32, CARD_STROKE));
    let inner = Rect::from_min_max(
        Pos2::new(card.left() + s_(28.0), card.top() + s_(22.0)),
        Pos2::new(card.right() - s_(28.0), card.bottom() - s_(22.0)),
    );
    let mut y = inner.top() + s_(6.0);
    let chip_text = ui_text(cfg, "welcome_chip").to_uppercase();
    let chip_w = s_(70.0).max(36.0);
    let chip_h = s_(24.0).max(14.0);
    p.rect_filled(
        Rect::from_min_size(Pos2::new(inner.left(), y), Vec2::new(chip_w, chip_h)),
        chip_h * 0.5,
        accent.linear_multiply(0.18),
    );
    p.text(
        Pos2::new(inner.left() + chip_w / 2.0, y + chip_h / 2.0),
        Align2::CENTER_CENTER,
        chip_text,
        FontId::proportional(s_(11.5)),
        accent,
    );
    y += chip_h + s_(16.0);
    p.text(
        Pos2::new(inner.left(), y),
        Align2::LEFT_TOP,
        &cfg.app.name,
        FontId::proportional(s_(40.0)),
        TEXT,
    );
    y += s_(52.0);
    p.text(
        Pos2::new(inner.left(), y),
        Align2::LEFT_TOP,
        format!(
            "版本 {} · {}",
            if cfg.app.version.is_empty() { "1.0.0" } else { &cfg.app.version },
            if cfg.app.publisher.is_empty() { "—" } else { &cfg.app.publisher }
        ),
        FontId::proportional(s_(15.0)),
        TEXT_DIM,
    );
    y += s_(30.0);
    let body = fill_template(&ui_text(cfg, "welcome_body"), &cfg.app.name, &cfg.app.version);
    let body_short: String = body.lines().take(2).collect::<Vec<_>>().join("\n");
    p.text(
        Pos2::new(inner.left(), y),
        Align2::LEFT_TOP,
        body_short,
        FontId::proportional(s_(14.5)),
        TEXT_DIM,
    );
    // 底部按钮（静态示意）
    let by = rect.bottom() - s_(32.0) - s_(21.0);
    let bx = rect.right() - s_(44.0);
    let next_w = s_(118.0);
    let next_h = s_(42.0);
    p.rect_filled(
        Rect::from_min_size(Pos2::new(bx - next_w, by), Vec2::new(next_w, next_h)),
        s_(10.0),
        accent,
    );
    p.text(
        Pos2::new(bx - next_w / 2.0, by + next_h / 2.0),
        Align2::CENTER_CENTER,
        ui_text(cfg, "next"),
        FontId::proportional(s_(15.0)),
        Color32::WHITE,
    );
    let cancel_w = s_(118.0);
    p.rect_filled(
        Rect::from_min_size(
            Pos2::new(bx - next_w - s_(12.0) - cancel_w, by),
            Vec2::new(cancel_w, next_h),
        ),
        s_(10.0),
        Color32::from_rgb(18, 21, 28),
    );
    p.text(
        Pos2::new(bx - next_w - s_(12.0) - cancel_w / 2.0, by + next_h / 2.0),
        Align2::CENTER_CENTER,
        ui_text(cfg, "cancel"),
        FontId::proportional(s_(15.0)),
        TEXT_DIM,
    );
}
