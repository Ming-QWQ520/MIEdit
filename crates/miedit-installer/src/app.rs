//! 安装向导 UI：欢迎 → 许可 → 组件 → 目录 → 安装中 → 完成/失败。
//! 无边框窗口 + 全窗口图片背景 + 半透明卡片，现代化视觉。
//! 功能对齐 Inno Setup：组件选择、磁盘空间检查、许可强制滚动、[Run] 任务、升级检测、运行中检测。

use crate::install::{self, Phase, ProgressState};
use crate::pkg::Package;
use crate::theme::{self, draw_badge, draw_progress};
use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, TextureHandle, Vec2, ViewportCommand,
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Page {
    Welcome,
    License,
    Components,
    Dir,
    Install,
    Finish,
    AppRunning,
    Error,
}

pub struct InstallerApp {
    pkg: Arc<Package>,
    dest: PathBuf,
    /// 目录选择页上的文本框内容
    dir_text: String,
    state: Arc<Mutex<ProgressState>>,
    page: Page,
    license_accepted: bool,
    license_scrolled: bool,
    component_checked: Vec<bool>,
    run_checked: Vec<bool>,
    err_msg: String,
    theme: theme::Theme,
    bg_tex: Option<TextureHandle>,
    /// 应用图标纹理（标题栏 + 欢迎页展示）
    icon_tex: Option<TextureHandle>,
    /// 安装页“显示细节”开关（默认展开）
    details_open: bool,
    title: String,
}

pub fn run_gui(pkg: Arc<Package>, default_dest: PathBuf) -> anyhow::Result<()> {
    let cfg = &pkg.config;
    let title = miedit_core::config::fill_template(
        &miedit_core::config::ui_text(cfg, "window_title"),
        &cfg.app.name,
        &cfg.app.version,
    );

    let [w, h] = cfg.ui.window_size;
    let w = w.max(720.0).min(1600.0);
    let h = h.max(520.0).min(1200.0);

    let mut viewport = egui::ViewportBuilder::default()
        .with_title(title.clone())
        .with_inner_size([w, h])
        .with_min_inner_size([w, h])
        .with_resizable(false)
        .with_decorations(false);
    if let Some(icon) = &pkg.icon {
        viewport = viewport.with_icon(icon.clone());
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    let run_title = title.clone();
    eframe::run_native(
        &run_title,
        options,
        Box::new(move |cc| {
            theme::install_fonts(&cc.egui_ctx);
            let th = theme::Theme::from_config(&pkg);
            th.apply(&cc.egui_ctx);
            let bg_tex = pkg
                .bg
                .as_ref()
                .map(|img| {
                    cc.egui_ctx
                        .load_texture("bg", img.clone(), egui::TextureOptions::LINEAR)
                });
            let icon_tex = pkg.icon.as_ref().map(|ic| {
                cc.egui_ctx.load_texture(
                    "app-icon",
                    egui::ColorImage::from_rgba_unmultiplied(
                        [ic.width as usize, ic.height as usize],
                        &ic.rgba,
                    ),
                    egui::TextureOptions::LINEAR,
                )
            });
            let component_checked = pkg.config.components.iter().map(|_| true).collect();
            Ok(Box::new(InstallerApp {
                pkg,
                dest: default_dest.clone(),
                dir_text: default_dest.display().to_string(),
                state: Arc::new(Mutex::new(ProgressState::default())),
                page: Page::Welcome,
                license_accepted: false,
                license_scrolled: false,
                component_checked,
                run_checked: Vec::new(),
                err_msg: String::new(),
                theme: th,
                bg_tex,
                icon_tex,
                details_open: true,
                title,
            }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("GUI 运行失败: {e}"))?;
    Ok(())
}

impl InstallerApp {
    // ---- 页面流转 ----

    fn has_license(&self) -> bool {
        self.pkg.config.license_shown()
    }
    fn has_dir_page(&self) -> bool {
        self.pkg.config.ui.show_dir_page
    }
    fn has_components(&self) -> bool {
        self.pkg.config.components_shown()
    }

    fn dest_display(&self) -> String {
        self.dest.display().to_string()
    }

    /// 组件选择 → 允许安装的规则索引集合；None = 全部安装。
    fn selected_rules(&self) -> Option<std::collections::HashSet<usize>> {
        if !self.has_components() {
            return None;
        }
        let cfg = &self.pkg.config;
        // 未被任何组件引用的规则始终安装
        let referenced: std::collections::HashSet<usize> = cfg
            .components
            .iter()
            .flat_map(|c| c.files.iter().copied())
            .collect();
        let mut set = std::collections::HashSet::new();
        for (i, c) in cfg.components.iter().enumerate() {
            if c.required || self.component_checked.get(i).copied().unwrap_or(true) {
                set.extend(c.files.iter().copied());
            }
        }
        for r in 0..cfg.files.len() {
            if !referenced.contains(&r) {
                set.insert(r);
            }
        }
        Some(set)
    }

    fn selected_bytes(&self) -> u64 {
        match self.selected_rules() {
            None => self.pkg.total_bytes,
            Some(rules) => {
                let mut total = 0u64;
                for r in rules {
                    if let Some(list) = self.pkg.config.files_map.get(r) {
                        for f in list {
                            total += f.size;
                        }
                    }
                }
                total
            }
        }
    }

    fn disk_free(&self) -> Option<u64> {
        miedit_core::winutil::disk_free_space(&self.dest)
    }

    fn check_app_running(&self) -> bool {
        if self.pkg.config.app.exe.is_empty() {
            return false;
        }
        let exe = self.dest.join(&self.pkg.config.app.exe);
        miedit_core::winutil::is_file_locked(&exe)
    }

    fn start_install(&mut self, ctx: &egui::Context) {
        let dest = PathBuf::from(self.dir_text.trim().trim_matches('"'));
        self.dest = dest.clone();

        // 主程序运行中检测（NSIS/Inno 常见预检）
        if self.check_app_running() {
            self.page = Page::AppRunning;
            return;
        }

        let rules = self.selected_rules();
        self.state = install::spawn(self.pkg.clone(), dest, Some(ctx.clone()), rules);
        self.page = Page::Install;
    }

    fn next(&mut self, ctx: &egui::Context) {
        match self.page {
            Page::Welcome => {
                if self.has_license() {
                    self.page = Page::License;
                } else if self.has_components() {
                    self.page = Page::Components;
                } else if self.has_dir_page() {
                    self.page = Page::Dir;
                } else {
                    self.start_install(ctx);
                }
            }
            Page::License => {
                if self.has_components() {
                    self.page = Page::Components;
                } else if self.has_dir_page() {
                    self.page = Page::Dir;
                } else {
                    self.start_install(ctx);
                }
            }
            Page::Components => {
                if self.has_dir_page() {
                    self.page = Page::Dir;
                } else {
                    self.start_install(ctx);
                }
            }
            Page::Dir => self.start_install(ctx),
            _ => {}
        }
    }

    fn back(&mut self) {
        match self.page {
            Page::License => self.page = Page::Welcome,
            Page::Components => {
                self.page = if self.has_license() { Page::License } else { Page::Welcome };
            }
            Page::Dir => {
                self.page = if self.has_components() {
                    Page::Components
                } else if self.has_license() {
                    Page::License
                } else {
                    Page::Welcome
                };
            }
            _ => {}
        }
    }

    fn poll_state(&mut self) {
        let s = self.state.lock().unwrap();
        match s.phase.clone() {
            Phase::Done => {
                if self.page != Page::Finish && self.page != Page::Error {
                    self.page = Page::Finish;
                }
            }
            Phase::Failed(e) => {
                if self.page != Page::Error {
                    self.page = Page::Error;
                    self.err_msg = e;
                }
            }
            _ => {}
        }
    }

    /// 完成页的任务列表（兼容旧 run_after 字段）。
    fn run_tasks(&self) -> Vec<(String, String, String)> {
        let cfg = &self.pkg.config;
        if !cfg.install.run_tasks.is_empty() {
            return cfg
                .install
                .run_tasks
                .iter()
                .map(|t| {
                    (
                        miedit_core::config::fill_template(&t.name, &cfg.app.name, &cfg.app.version),
                        t.exe.clone(),
                        t.args.clone(),
                    )
                })
                .collect();
        }
        let exe = if cfg.install.run_after.is_empty() {
            cfg.app.exe.clone()
        } else {
            cfg.install.run_after.clone()
        };
        if exe.is_empty() {
            return Vec::new();
        }
        let name = miedit_core::config::fill_template(
            &miedit_core::config::ui_text(cfg, "run_app"),
            &cfg.app.name,
            &cfg.app.version,
        );
        vec![(name, exe, String::new())]
    }

    // ---- 主操作（NSIS：Enter 触发默认按钮 / Esc 取消）----

    fn primary_action(&mut self, ctx: &egui::Context) {
        match self.page {
            Page::Welcome | Page::License | Page::Components | Page::Dir => {
                let enabled = match self.page {
                    Page::License => self.license_accepted,
                    Page::Dir => match self.disk_free() {
                        Some(free) => free >= self.selected_bytes(),
                        None => true,
                    },
                    _ => true,
                };
                if enabled {
                    self.next(ctx);
                }
            }
            Page::Finish => {
                let tasks = self.run_tasks();
                let chosen: Vec<(String, String)> = tasks
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| self.run_checked.get(*i).copied().unwrap_or(false))
                    .map(|(_, (name, exe, args))| (name.clone(), format!("{}\u{1}{}", exe, args)))
                    .collect();
                for (_, packed) in &chosen {
                    if let Some((exe, args)) = packed.split_once('\u{1}') {
                        let target = self.dest.join(exe);
                        if args.is_empty() {
                            let _ = miedit_core::winutil::shell_open(&target);
                        } else {
                            let _ = miedit_core::winutil::shell_open_with_args(&target, args);
                        }
                    }
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Page::AppRunning => {
                if !self.check_app_running() {
                    self.start_install(ctx);
                }
            }
            Page::Error => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Page::Install => {}
        }
    }

    // ---- 卡片内容 ----

    fn content(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        let cfg = &self.pkg.config;
        match self.page {
            Page::Welcome => {
                let chip = miedit_core::config::ui_text(cfg, "welcome_chip");
                ui.add_space(6.0);
                chip_label(ui, &chip, self.theme.accent);
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if let Some(icon) = &self.icon_tex {
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(56.0), Sense::hover());
                        ui.painter().image(
                            icon.id(),
                            rect,
                            Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                        ui.add_space(6.0);
                    }
                    ui.heading(
                        egui::RichText::new(&cfg.app.name)
                            .color(self.theme.text)
                            .size(40.0)
                            .strong(),
                    );
                });
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(format!(
                        "版本 {} · {}",
                        cfg.app.version,
                        if cfg.app.publisher.is_empty() { "—" } else { &cfg.app.publisher }
                    ))
                    .color(self.theme.text_dim)
                    .size(15.0),
                );
                ui.add_space(16.0);
                let body = miedit_core::config::fill_template(
                    &miedit_core::config::ui_text(cfg, "welcome_body"),
                    &cfg.app.name,
                    &cfg.app.version,
                );
                ui.label(egui::RichText::new(body).color(self.theme.text_dim).size(14.5));
            }
            Page::License => {
                ui.heading(
                    egui::RichText::new(miedit_core::config::ui_text(cfg, "license_title"))
                        .color(self.theme.text)
                        .size(28.0)
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(miedit_core::config::ui_text(cfg, "license_desc"))
                        .color(self.theme.text_dim)
                        .size(14.0),
                );
                ui.add_space(8.0);
                // 复选框 + 间距预留，避免内容溢出卡片
                let reserve = 56.0;
                let box_h = (ui.available_height() - reserve).max(120.0);
                let scroll_required = cfg.ui.license_scroll_required;
                let mut reached_bottom = !scroll_required;
                egui::Frame::none()
                    .fill(self.theme.field)
                    .rounding(10.0)
                    .stroke(Stroke::new(1.0, self.theme.card_stroke))
                    .inner_margin(egui::Margin::same(14.0))
                    .show(ui, |ui| {
                        let out = egui::ScrollArea::vertical()
                            .auto_shrink(false)
                            .max_height(box_h - 28.0)
                            .show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                ui.label(
                                    egui::RichText::new(
                                        self.pkg.config.ui.license_text.as_deref().unwrap_or(""),
                                    )
                                    .color(self.theme.text_dim)
                                    .size(13.5),
                                );
                            });
                        // Inno 风格：强制滚动到底部才能接受
                        let content_h = out.content_size.y;
                        let view_h = out.inner_rect.height();
                        let offset_y = out.state.offset.y;
                        if content_h <= view_h + 2.0 || offset_y >= content_h - view_h - 4.0 {
                            reached_bottom = true;
                        }
                    });
                self.license_scrolled = reached_bottom;
                ui.add_space(6.0);
                let can_accept = !scroll_required || self.license_scrolled;
                ui.add_enabled_ui(can_accept, |ui| {
                    ui.checkbox(
                        &mut self.license_accepted,
                        egui::RichText::new(miedit_core::config::ui_text(cfg, "license_accept"))
                            .color(if can_accept { self.theme.text } else { self.theme.text_dim })
                            .size(14.0),
                    );
                });
                if scroll_required && !self.license_scrolled {
                    ui.label(
                        egui::RichText::new("（请先将许可协议滚动到底部）")
                            .color(self.theme.text_dim)
                            .size(12.0),
                    );
                }
            }
            Page::Components => {
                ui.heading(
                    egui::RichText::new(miedit_core::config::ui_text(cfg, "components_title"))
                        .color(self.theme.text)
                        .size(28.0)
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(miedit_core::config::fill_template(
                        &miedit_core::config::ui_text(cfg, "components_desc"),
                        &cfg.app.name,
                        &cfg.app.version,
                    ))
                    .color(self.theme.text_dim)
                    .size(14.0),
                );
                ui.add_space(8.0);
                let mut checked_next = self.component_checked.clone();
                ui.horizontal(|ui| {
                    if ui
                        .add(theme::secondary_button(
                            &miedit_core::config::ui_text(cfg, "components_select_all"),
                            &self.theme,
                        ))
                        .clicked()
                    {
                        for c in checked_next.iter_mut() {
                            *c = true;
                        }
                    }
                    if ui
                        .add(theme::secondary_button(
                            &miedit_core::config::ui_text(cfg, "components_reset"),
                            &self.theme,
                        ))
                        .clicked()
                    {
                        for (i, c) in self.pkg.config.components.iter().enumerate() {
                            if let Some(v) = checked_next.get_mut(i) {
                                *v = c.required;
                            }
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(miedit_core::util::human_size(self.selected_bytes()))
                                .color(self.theme.text_dim)
                                .size(12.5),
                        );
                    });
                });
                ui.add_space(4.0);
                egui::Frame::none()
                    .fill(self.theme.field)
                    .rounding(10.0)
                    .stroke(Stroke::new(1.0, self.theme.card_stroke))
                    .inner_margin(egui::Margin::same(14.0))
                    .show(ui, |ui| {
                        for (i, comp) in self.pkg.config.components.iter().enumerate() {
                            let checked = checked_next.get(i).copied().unwrap_or(true);
                            let mut next = checked;
                            ui.add_enabled_ui(!comp.required, |ui| {
                                ui.checkbox(
                                    &mut next,
                                    egui::RichText::new(&comp.name)
                                        .color(self.theme.text)
                                        .size(15.0)
                                        .strong(),
                                );
                            });
                            checked_next[i] = if comp.required { true } else { next };
                            if !comp.description.is_empty() {
                                ui.horizontal(|ui| {
                                    ui.add_space(22.0);
                                    ui.label(
                                        egui::RichText::new(&comp.description)
                                            .color(self.theme.text_dim)
                                            .size(12.5),
                                    );
                                });
                            }
                            if comp.required {
                                ui.horizontal(|ui| {
                                    ui.add_space(22.0);
                                    chip_small(
                                        ui,
                                        miedit_core::config::ui_text(cfg, "components_required"),
                                        self.theme.accent,
                                    );
                                });
                            }
                            ui.add_space(6.0);
                        }
                    });
                self.component_checked = checked_next;
            }
            Page::Dir => {
                ui.heading(
                    egui::RichText::new(miedit_core::config::ui_text(cfg, "dir_title"))
                        .color(self.theme.text)
                        .size(28.0)
                        .strong(),
                );
                ui.add_space(4.0);
                let desc = miedit_core::config::fill_template(
                    &miedit_core::config::ui_text(cfg, "dir_desc"),
                    &cfg.app.name,
                    &cfg.app.version,
                );
                ui.label(egui::RichText::new(desc).color(self.theme.text_dim).size(14.5));
                ui.add_space(14.0);

                let row_avail = ui.available_width();
                ui.horizontal(|ui| {
                    egui::Frame::none()
                        .fill(self.theme.field)
                        .rounding(9.0)
                        .stroke(Stroke::new(1.0, self.theme.card_stroke))
                        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
                        .show(ui, |ui| {
                            ui.add_sized(
                                [row_avail - 138.0, 22.0],
                                egui::TextEdit::singleline(&mut self.dir_text)
                                    .font(egui::TextStyle::Body)
                                    .frame(false),
                            );
                        });
                    if ui
                        .add(theme::secondary_button(
                            &miedit_core::config::ui_text(cfg, "browse"),
                            &self.theme,
                        ))
                        .clicked()
                    {
                        if let Some(dir) = rfd::FileDialog::new()
                            .set_title(miedit_core::config::ui_text(cfg, "dir_title"))
                            .pick_folder()
                        {
                            self.dir_text = dir.display().to_string();
                        }
                    }
                });

                ui.add_space(6.0);
                let need = self.selected_bytes();
                let space =
                    miedit_core::config::ui_text(cfg, "disk_space").replace("{size}", &miedit_core::util::human_size(need));
                ui.label(egui::RichText::new(space).color(self.theme.text_dim).size(13.0));
                if let Some(free) = self.disk_free() {
                    let free_s = miedit_core::config::ui_text(cfg, "free_space")
                        .replace("{free}", &miedit_core::util::human_size(free));
                    let insufficient = free < need.saturating_mul(2);
                    ui.label(
                        egui::RichText::new(free_s)
                            .color(if insufficient {
                                Color32::from_rgb(0xE5, 0x48, 0x4D)
                            } else {
                                self.theme.text_dim
                            })
                            .size(13.0),
                    );
                }
                // Inno DirExistsWarning：目标目录已存在且非空时提示
                if self.dir_text.trim().trim_matches('"').len() > 0 {
                    let d = PathBuf::from(self.dir_text.trim().trim_matches('"'));
                    if d.is_dir() {
                        let non_empty = std::fs::read_dir(&d)
                            .map(|mut it| it.next().is_some())
                            .unwrap_or(false);
                        if non_empty {
                            ui.add_space(2.0);
                            ui.label(
                                egui::RichText::new("注意：目标目录已存在且非空，其中的文件将被覆盖或合并。")
                                    .color(Color32::from_rgb(0xF5, 0xA5, 0x24))
                                    .size(12.5),
                            );
                        }
                    }
                }
            }
            Page::Install => {
                ui.heading(
                    egui::RichText::new(miedit_core::config::fill_template(
                        &miedit_core::config::ui_text(cfg, "installing"),
                        &cfg.app.name,
                        &cfg.app.version,
                    ))
                    .color(self.theme.text)
                    .size(28.0)
                    .strong(),
                );
                ui.add_space(10.0);
                let (frac, cur, log, phase, done_f, total_f, done_b, speed_mb) = {
                    let s = self.state.lock().unwrap();
                    let total = if s.total_bytes == 0 { 1 } else { s.total_bytes };
                    let elapsed = s
                        .started
                        .map(|t| t.elapsed().as_secs_f64())
                        .unwrap_or(1.0)
                        .max(0.001);
                    let mb_s = (s.done_bytes as f64 / 1024.0 / 1024.0) / elapsed;
                    (
                        s.done_bytes as f32 / total as f32,
                        s.current_file.clone(),
                        s.log.clone(),
                        s.phase.clone(),
                        s.done_files,
                        s.total_files,
                        s.done_bytes,
                        mb_s,
                    )
                };
                let pct = (frac * 100.0).min(100.0);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("{pct:.0}%"))
                            .color(self.theme.accent)
                            .size(30.0)
                            .strong(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(match phase {
                                Phase::Extracting | Phase::Idle => {
                                    miedit_core::config::ui_text(cfg, "preparing")
                                }
                                _ => miedit_core::config::ui_text(cfg, "finalizing"),
                            })
                            .color(self.theme.text_dim)
                            .size(13.5),
                        );
                    });
                });
                draw_progress(ui, frac, self.theme.accent, 9.0);
                ui.add_space(6.0);
                if !cur.is_empty() {
                    ui.label(
                        egui::RichText::new(
                            miedit_core::config::ui_text(cfg, "extracting").replace("{file}", &cur),
                        )
                        .color(self.theme.text_dim)
                        .size(12.5),
                    );
                }
                // 细节：文件计数 + 速度 + 日志开关（NSIS “Show details” 风格，默认展开）
                ui.horizontal(|ui| {
                    let speed_s = if speed_mb >= 1.0 {
                        format!("{speed_mb:.1} MB/s")
                    } else {
                        format!("{:.0} KB/s", speed_mb * 1024.0)
                    };
                    ui.label(
                        egui::RichText::new(format!(
                            "已写入 {} / {} 个文件 · {} · {}",
                            done_f,
                            total_f,
                            miedit_core::util::human_size(done_b),
                            speed_s
                        ))
                        .color(self.theme.text_dim)
                        .size(12.5),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = if self.details_open { "隐藏细节" } else { "显示细节" };
                        if ui
                            .add(theme::secondary_button(label, &self.theme))
                            .clicked()
                        {
                            self.details_open = !self.details_open;
                        }
                    });
                });
                if self.details_open {
                    egui::Frame::none()
                        .fill(self.theme.field)
                        .rounding(10.0)
                        .stroke(Stroke::new(1.0, self.theme.card_stroke))
                        .inner_margin(egui::Margin::same(10.0))
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .auto_shrink(false)
                                .stick_to_bottom(true)
                                .show(ui, |ui| {
                                    ui.set_min_width(ui.available_width());
                                    for line in &log {
                                        ui.label(
                                            egui::RichText::new(line)
                                                .color(self.theme.text_dim)
                                                .size(12.0)
                                                .monospace(),
                                        );
                                    }
                                });
                        });
                }
            }
            Page::Finish => {
                ui.add_space(4.0);
                draw_badge(ui, Color32::from_rgb(0x3D, 0xD6, 0x8C), "√", 34.0);
                ui.add_space(10.0);
                ui.heading(
                    egui::RichText::new(miedit_core::config::ui_text(cfg, "finish_title"))
                        .color(self.theme.text)
                        .size(34.0)
                        .strong(),
                );
                ui.add_space(4.0);
                let body = miedit_core::config::fill_template(
                    &miedit_core::config::ui_text(cfg, "finish_body"),
                    &cfg.app.name,
                    &cfg.app.version,
                );
                ui.label(egui::RichText::new(body).color(self.theme.text_dim).size(15.0));
                ui.label(
                    egui::RichText::new(self.dest_display())
                        .color(self.theme.text)
                        .size(15.0)
                        .strong(),
                );
                let tasks = self.run_tasks();
                if !tasks.is_empty() {
                    ui.add_space(10.0);
                    if self.run_checked.len() != tasks.len() {
                        if cfg.install.run_tasks.len() == tasks.len() {
                            self.run_checked = cfg
                                .install
                                .run_tasks
                                .iter()
                                .map(|t| t.default_checked)
                                .collect();
                        } else {
                            self.run_checked = vec![true; tasks.len()];
                        }
                    }
                    for (i, (name, _, _)) in tasks.iter().enumerate() {
                        let mut v = self.run_checked.get(i).copied().unwrap_or(true);
                        ui.checkbox(
                            &mut v,
                            egui::RichText::new(name).color(self.theme.text).size(14.5),
                        );
                        if let Some(slot) = self.run_checked.get_mut(i) {
                            *slot = v;
                        }
                    }
                }
            }
            Page::AppRunning => {
                ui.add_space(4.0);
                draw_badge(ui, Color32::from_rgb(0xF5, 0xA5, 0x24), "!", 34.0);
                ui.add_space(10.0);
                ui.heading(
                    egui::RichText::new(miedit_core::config::ui_text(cfg, "app_running_title"))
                        .color(self.theme.text)
                        .size(30.0)
                        .strong(),
                );
                ui.add_space(6.0);
                let body = miedit_core::config::fill_template(
                    &miedit_core::config::ui_text(cfg, "app_running_body"),
                    &cfg.app.name,
                    &cfg.app.version,
                );
                ui.label(egui::RichText::new(body).color(self.theme.text_dim).size(14.5));
            }
            Page::Error => {
                ui.add_space(4.0);
                draw_badge(ui, self.theme.accent, "!", 34.0);
                ui.add_space(10.0);
                ui.heading(
                    egui::RichText::new(miedit_core::config::ui_text(cfg, "error_title"))
                        .color(self.theme.text)
                        .size(30.0)
                        .strong(),
                );
                ui.add_space(6.0);
                egui::Frame::none()
                    .fill(self.theme.field)
                    .rounding(10.0)
                    .stroke(Stroke::new(1.0, self.theme.card_stroke))
                    .inner_margin(egui::Margin::same(12.0))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.label(
                            egui::RichText::new(&self.err_msg)
                                .color(self.theme.text_dim)
                                .size(13.5),
                        );
                    });
            }
        }
    }

    // ---- 底部按钮 ----

    fn buttons(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let cfg = self.pkg.config.clone();
        let t_next = miedit_core::config::ui_text(&cfg, "next");
        let t_back = miedit_core::config::ui_text(&cfg, "back");
        let t_cancel = miedit_core::config::ui_text(&cfg, "cancel");
        let t_install = miedit_core::config::ui_text(&cfg, "install_now");
        let t_done = miedit_core::config::ui_text(&cfg, "done_btn");
        let t_retry = miedit_core::config::ui_text(&cfg, "retry");
        let t_continue = miedit_core::config::ui_text(&cfg, "continue_anyway");

        ui.horizontal(|ui| {
            match self.page {
                Page::Welcome | Page::License | Page::Components | Page::Dir => {
                    if ui.add(theme::secondary_button(&t_cancel, &self.theme)).clicked() {
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                    }
                    ui.add_space(4.0);
                    if self.page != Page::Welcome {
                        if ui.add(theme::secondary_button(&t_back, &self.theme)).clicked() {
                            self.back();
                        }
                        ui.add_space(4.0);
                    }
                    let enabled = match self.page {
                        Page::License => self.license_accepted,
                        Page::Dir => match self.disk_free() {
                            Some(free) => free >= self.selected_bytes(),
                            None => true,
                        },
                        _ => true,
                    };
                    let is_last = match self.page {
                        Page::Dir => true,
                        Page::License => !self.has_components() && !self.has_dir_page(),
                        Page::Components => !self.has_dir_page(),
                        Page::Welcome => !self.has_license() && !self.has_components() && !self.has_dir_page(),
                        _ => false,
                    };
                    let label: &str = if is_last { &t_install } else { &t_next };
                    if ui
                        .add_enabled(enabled, theme::primary_button(label, &self.theme))
                        .clicked()
                    {
                        self.primary_action(ctx);
                    }
                }
                Page::Install => {}
                Page::Finish => {
                    if ui.add(theme::primary_button(&t_done, &self.theme)).clicked() {
                        self.primary_action(ctx);
                    }
                }
                Page::AppRunning => {
                    if ui.add(theme::secondary_button(&t_continue, &self.theme)).clicked() {
                        let dest = PathBuf::from(self.dir_text.trim().trim_matches('"'));
                        self.dest = dest.clone();
                        let rules = self.selected_rules();
                        self.state =
                            install::spawn(self.pkg.clone(), dest, Some(ctx.clone()), rules);
                        self.page = Page::Install;
                    }
                    ui.add_space(4.0);
                    if ui.add(theme::primary_button(&t_retry, &self.theme)).clicked() {
                        self.primary_action(ctx);
                    }
                }
                Page::Error => {
                    if ui.add(theme::primary_button(&t_done, &self.theme)).clicked() {
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                    }
                }
            }
        });
    }
}

fn chip_small(ui: &mut egui::Ui, text: String, accent: Color32) {
    egui::Frame::none()
        .fill(accent.linear_multiply(0.16))
        .stroke(Stroke::new(1.0, accent.linear_multiply(0.5)))
        .rounding(999.0)
        .inner_margin(egui::Margin::symmetric(8.0, 2.0))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).color(accent).size(11.0).strong());
        });
}

fn chip_label(ui: &mut egui::Ui, text: &str, accent: Color32) {
    let frame = egui::Frame::none()
        .fill(accent.linear_multiply(0.18))
        .rounding(999.0)
        .inner_margin(egui::Margin::symmetric(12.0, 5.0))
        .stroke(Stroke::new(1.0, accent.linear_multiply(0.55)));
    frame.show(ui, |ui| {
        ui.label(
            egui::RichText::new(text.to_uppercase())
                .color(accent)
                .size(11.5)
                .strong()
                .extra_letter_spacing(2.0),
        );
    });
}

impl eframe::App for InstallerApp {
    #[allow(deprecated)]
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // NSIS 操作方式：Enter 触发主操作，Esc 取消（安装进行中忽略 Esc 防误触）
        let (enter, esc) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if esc && self.page != Page::Install {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if enter && self.page != Page::Install {
            self.primary_action(ctx);
        }
        self.poll_state();

        egui::CentralPanel::default()
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                let sr = ui.max_rect();
                theme::paint_background(
                    ui.painter(),
                    sr,
                    self.bg_tex.as_ref(),
                    &self.pkg.config.ui.background_style,
                    self.theme.accent,
                );

                // 无边框标题栏：拖拽 + 最小化/关闭
                let tb = Rect::from_min_size(sr.min, Vec2::new(sr.width(), 44.0));
                let drag = ui.interact(tb, egui::Id::new("drag"), Sense::click_and_drag());
                if drag.dragged() {
                    ctx.send_viewport_cmd(ViewportCommand::StartDrag);
                }
                let mut title_x = sr.left() + 18.0;
                if let Some(icon) = &self.icon_tex {
                    let icon_rect = Rect::from_min_size(
                        Pos2::new(sr.left() + 16.0, sr.top() + 12.0),
                        Vec2::splat(20.0),
                    );
                    ui.painter().image(
                        icon.id(),
                        icon_rect,
                        Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    title_x = sr.left() + 44.0;
                }
                ui.painter().text(
                    Pos2::new(title_x, sr.top() + 22.0),
                    Align2::LEFT_CENTER,
                    &self.title,
                    FontId::proportional(13.5),
                    self.theme.text_dim,
                );
                // 关闭
                let close_rect =
                    Rect::from_min_size(Pos2::new(sr.right() - 44.0, sr.top() + 8.0), Vec2::new(36.0, 28.0));
                let crect = ui.interact(close_rect, egui::Id::new("win-close"), Sense::click());
                if crect.hovered() {
                    ui.painter().rect_filled(close_rect, 7.0, Color32::from_rgb(196, 30, 58));
                }
                let c = close_rect.center();
                let (s1, s2) = (Vec2::new(5.0, 5.0), Vec2::new(-5.0, 5.0));
                ui.painter().line_segment([c - s1, c + s1], Stroke::new(1.6, self.theme.text_dim));
                ui.painter().line_segment([c - s2, c + s2], Stroke::new(1.6, self.theme.text_dim));
                if crect.clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
                // 最小化
                let min_rect =
                    Rect::from_min_size(Pos2::new(sr.right() - 82.0, sr.top() + 8.0), Vec2::new(36.0, 28.0));
                let mrect = ui.interact(min_rect, egui::Id::new("win-min"), Sense::click());
                if mrect.hovered() {
                    ui.painter().rect_filled(min_rect, 7.0, Color32::from_white_alpha(26));
                }
                let m = min_rect.center();
                ui.painter().line_segment(
                    [Pos2::new(m.x - 5.0, m.y), Pos2::new(m.x + 5.0, m.y)],
                    Stroke::new(1.6, self.theme.text_dim),
                );
                if mrect.clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
                }

                // 半透明卡片
                let card = Rect::from_min_max(
                    Pos2::new(sr.left() + 44.0, sr.top() + 108.0),
                    Pos2::new(sr.right() - 44.0, sr.bottom() - 104.0),
                );
                ui.painter().rect_filled(card, 18.0, self.theme.card.linear_multiply(0.62));
                ui.painter().rect(
                    card,
                    18.0,
                    Color32::TRANSPARENT,
                    Stroke::new(1.0, self.theme.card_stroke),
                );
                ui.allocate_ui_at_rect(card.shrink2(Vec2::new(28.0, 22.0)), |ui| {
                    self.content(ui, ctx);
                });
            });

        // 底部按钮（浮动）
        egui::Area::new(egui::Id::new("buttons"))
            .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-44.0, -32.0))
            .show(ctx, |ui| {
                self.buttons(ui, ctx);
            });
    }
}
