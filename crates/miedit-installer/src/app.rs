//! 安装向导 UI：欢迎 → 许可 → 组件 → 目录 → 安装中 → 完成/失败。
//! 无边框窗口 + 全窗口图片背景 + 半透明卡片，现代化视觉。
//! 功能对齐 Inno Setup：组件选择、磁盘空间检查、许可强制滚动、[Run] 任务、升级检测、运行中检测、
//! ARP 修复入口（--repair 确认页，路线图步骤 17）。
//! 功能对齐 NSIS DirectoryPage（路线图步骤 20）：目录页可用空间随所选驱动器
//! 实时刷新，无效路径/根目录安装校验（allow_root_dir_install）。

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
    /// 语言选择页（NSIS MUI 语义：定义了多个语言时显示，欢迎页前）
    Language,
    Welcome,
    License,
    Components,
    Dir,
    /// 修复模式确认页（ARP ModifyPath 入口，路线图步骤 17；
    /// Inno 修复向导语义：确认后全量重装 + 重建快捷方式/注册表）
    RepairConfirm,
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
    /// 当前选中语言索引（cfg.ui.languages 下标；None = 无语言包，旧包行为）
    language: Option<usize>,
    license_accepted: bool,
    license_scrolled: bool,
    component_checked: Vec<bool>,
    /// 当前选中的安装类型索引（NSIS InstallTypes）；`types.len()` = Custom
    install_type: usize,
    /// 组件页描述面板跟随的组件索引（悬停/点击更新，NSIS 描述框语义）
    desc_index: Option<usize>,
    /// 组件分组树已折叠组（步骤 24：NSIS SectionGroup 展开态；空集 = 全部展开）
    comp_collapsed: std::collections::BTreeSet<String>,
    run_checked: Vec<bool>,
    err_msg: String,
    theme: theme::Theme,
    bg_tex: Option<TextureHandle>,
    /// 应用图标纹理（标题栏 + 欢迎页展示）
    icon_tex: Option<TextureHandle>,
    /// 安装页“显示细节”开关（默认展开）
    details_open: bool,
    title: String,
    /// 修复模式（ARP ModifyPath 启动，路线图步骤 17）：
    /// 跳过向导页直接确认 → 全量重装，完成页切换为修复文案
    repair: bool,
    /// 步骤 18：安装完成（后台线程 Done）——停留在安装详情页（NSIS
    /// INSTFILES 完成态语义），点「下一步」才进入完成页
    install_finished: bool,
    /// 步骤 21（CLI 适配）：--no-shortcuts 跳过快捷方式创建
    no_shortcuts: bool,
    /// 步骤 30：后台 CRC 校验门（提取前等待校验结果，窗口不阻塞）
    crc_gate: Arc<install::CrcGate>,
    /// 步骤 30：后台壁纸解码结果（解码完成后由 UI 线程上传纹理）
    bg_shared: Arc<Mutex<Option<egui::ColorImage>>>,
}

pub fn run_gui(
    pkg: Arc<Package>,
    default_dest: PathBuf,
    repair: bool,
    cli_components: Option<Vec<bool>>,
    no_shortcuts: bool,
    ncrc: bool,
) -> anyhow::Result<()> {
    let cfg = &pkg.config;
    // 启动语言预选（NSIS MUI：多语言时预选系统语言，单语言直接使用，无语言包保持旧行为）
    let language = miedit_core::langs::initial_language(cfg);
    let title = miedit_core::config::fill_template(
        &miedit_core::config::resolved_text(cfg, language, "window_title"),
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

    // 步骤 21：Windows 11 亚克力背景板（ui.acrylic；低版本失败静默回退
    // 为绘制的玻璃卡片，观感不受损）。窗口背景透明化以便 backdrop 透出
    // （必须在构建 options 前设置 viewport）。
    #[cfg(windows)]
    if theme::Theme::from_config(&pkg).acrylic {
        viewport = viewport.with_transparent(true);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    // 亚克力背景板：窗口创建后设置 DWM SystemBackdrop（异步轮询）
    #[cfg(windows)]
    if theme::Theme::from_config(&pkg).acrylic {
        miedit_core::winutil::enable_acrylic_backdrop(&title);
    }

    let run_title = title.clone();
    // 步骤 30（安装包打开速度）：CRC 校验与壁纸解码全部移入后台线程——
    // 窗口打开只依赖配置解析与图标（小）；校验在提取前完成（CrcGate），
    // 壁纸解码完成后下一帧上传纹理（渐进式呈现，先渐变后壁纸）
    let crc_gate = if ncrc {
        install::CrcGate::ready(Ok(()))
    } else {
        let gate = Arc::new(install::CrcGate::new_pending());
        let g2 = gate.clone();
        let pkg2 = pkg.clone();
        std::thread::Builder::new()
            .name("miedit-crc".into())
            .spawn(move || {
                let res = miedit_core::trailer::verify_crc(&pkg2.path, &pkg2.trailer)
                    .map_err(|e| format!("{e:#}"));
                g2.complete(res);
            })
            .expect("启动 CRC 校验线程失败");
        gate
    };
    // 步骤 30：壁纸解码结果共享槽（线程在窗口创建闭包内启动，以便拿到 ctx repaint）
    let bg_shared: Arc<Mutex<Option<egui::ColorImage>>> = Arc::new(Mutex::new(None));
    eframe::run_native(
        &run_title,
        options,
        Box::new(move |cc| {
            theme::install_fonts(&cc.egui_ctx);
            // 步骤 30：壁纸后台解码线程（IO + PNG 解码离关键路径；完成后
            // request_repaint 让空闲中的 UI 也能换上壁纸）
            if let Some(raw) = pkg.bg_raw.clone() {
                let s2 = bg_shared.clone();
                let ctx2 = cc.egui_ctx.clone();
                std::thread::Builder::new()
                    .name("miedit-bg".into())
                    .spawn(move || {
                        let img = crate::pkg::decode_image_color(&raw);
                        *s2.lock().unwrap() = img;
                        ctx2.request_repaint();
                    })
                    .expect("启动壁纸解码线程失败");
            }
            let th = theme::Theme::from_config(&pkg);
            th.apply(&cc.egui_ctx);
            // 步骤 30：壁纸初始为 None（后台解码完成后 update() 逐帧上传纹理）
            let bg_tex: Option<TextureHandle> = None;
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
            let types = &pkg.config.install_types;
            let mut install_type = miedit_core::installtypes::default_type_index(types)
                .unwrap_or(0); // 无类型时占位，组件页不渲染类型行
            let component_checked = match &cli_components {
                // 步骤 21：CLI --components= 预选组件（GUI 同样生效），
                // 类型状态切到 Custom（NSIS 自定义选择语义）
                Some(sel) => {
                    let v = pkg
                        .config
                        .components
                        .iter()
                        .enumerate()
                        .map(|(i, c)| c.required || sel.get(i).copied().unwrap_or(false))
                        .collect::<Vec<bool>>();
                    install_type = types.len(); // Custom
                    v
                }
                None => match miedit_core::installtypes::default_type_index(types) {
                    Some(i) => miedit_core::installtypes::selection_for_type(
                        types,
                        &pkg.config.components,
                        i,
                    )
                    .unwrap_or_else(|| pkg.config.components.iter().map(|_| true).collect()),
                    None => pkg.config.components.iter().map(|_| true).collect(),
                },
            };
            // NSIS MUI 语义：定义了多个语言 → 首页为语言选择页；
            // 修复模式跳过全部向导页，直接进入修复确认页
            let page = if repair {
                Page::RepairConfirm
            } else if pkg.config.ui.languages.len() > 1 {
                Page::Language
            } else {
                Page::Welcome
            };
            Ok(Box::new(InstallerApp {
                pkg,
                dest: default_dest.clone(),
                dir_text: default_dest.display().to_string(),
                state: Arc::new(Mutex::new(ProgressState::default())),
                page,
                language,
                license_accepted: false,
                license_scrolled: false,
                component_checked,
                comp_collapsed: std::collections::BTreeSet::new(),
                install_type,
                desc_index: None,
                run_checked: Vec::new(),
                err_msg: String::new(),
                theme: th,
                bg_tex,
                icon_tex,
                details_open: true,
                title,
                repair,
                install_finished: false,
                no_shortcuts,
                crc_gate: crc_gate.clone(),
                bg_shared,
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

    // ---- 步骤导航栏（HM NIS Edit 向导式，路线图步骤 21）----

    fn show_step_rail(&self) -> bool {
        self.pkg.config.ui.show_step_rail
    }

    /// 当前步骤对应的导航页（AppRunning 归入安装步；非向导页返回 None = 隐藏导航栏）。
    fn rail_current(&self) -> Option<Page> {
        match self.page {
            Page::AppRunning => Some(Page::Install),
            p @ (Page::Welcome
            | Page::License
            | Page::Components
            | Page::Dir
            | Page::Install
            | Page::Finish) => Some(p),
            _ => None,
        }
    }

    /// 绘制左列步骤导航（仅指示，不可点击——NSIS 页面流线性不可跳跃）：
    /// 当前步 accent 高亮 + 描边，已完成步勾点，未到步空心点。
    fn paint_step_rail(&self, ui: &egui::Ui, area: Rect) {
        let cfg = &self.pkg.config;
        let mut steps: Vec<(Page, &str)> = vec![(Page::Welcome, "nav_welcome")];
        if self.has_license() {
            steps.push((Page::License, "nav_license"));
        }
        if self.has_components() {
            steps.push((Page::Components, "nav_components"));
        }
        if self.has_dir_page() {
            steps.push((Page::Dir, "nav_dir"));
        }
        steps.push((Page::Install, "nav_install"));
        steps.push((Page::Finish, "nav_finish"));
        let Some(cur) = self.rail_current() else {
            return;
        };
        let cur_idx = steps
            .iter()
            .position(|(p, _)| *p == cur)
            .unwrap_or(usize::MAX);
        let painter = ui.painter();
        let row_h = 34.0;
        let mut y = area.top() + 18.0;
        for (i, (_page, key)) in steps.iter().enumerate() {
            let row = Rect::from_min_max(
                Pos2::new(area.left() + 10.0, y),
                Pos2::new(area.right() - 6.0, y + row_h),
            );
            let is_cur = i == cur_idx;
            let is_done = i < cur_idx;
            if is_cur {
                painter.rect_filled(
                    row,
                    9.0,
                    Color32::from_rgba_unmultiplied(
                        self.theme.accent.r(),
                        self.theme.accent.g(),
                        self.theme.accent.b(),
                        48,
                    ),
                );
                painter.rect(
                    row,
                    9.0,
                    Color32::TRANSPARENT,
                    Stroke::new(
                        1.0_f32,
                        Color32::from_rgba_unmultiplied(
                            self.theme.accent.r(),
                            self.theme.accent.g(),
                            self.theme.accent.b(),
                            130,
                        ),
                    ),
                );
            }
            // 状态点：当前 = 实心 accent；已完成 = accent 勾点；未到 = 空心
            let dot_c = Pos2::new(row.left() + 15.0, row.center().y);
            if is_cur {
                painter.circle_filled(dot_c, 4.5, self.theme.accent);
            } else if is_done {
                painter.circle_filled(dot_c, 4.5, self.theme.accent);
                painter.text(
                    Pos2::new(dot_c.x, dot_c.y - 0.5),
                    Align2::CENTER_CENTER,
                    "√",
                    FontId::proportional(7.5),
                    Color32::WHITE,
                );
            } else {
                painter.circle(
                    dot_c,
                    3.5,
                    Color32::TRANSPARENT,
                    Stroke::new(1.2_f32, self.theme.text_dim.linear_multiply(0.55)),
                );
            }
            let label = txt(cfg, self.language, key);
            painter.text(
                Pos2::new(row.left() + 30.0, row.center().y),
                Align2::LEFT_CENTER,
                label,
                FontId::proportional(if is_cur { 13.0 } else { 12.5 }),
                if is_cur {
                    self.theme.text
                } else {
                    self.theme.text_dim
                },
            );
            y += row_h + 4.0;
        }
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

    /// 当前生效的目标目录：目录页实时跟随输入框（dir_text），
    /// 而非仅在 start_install 时才更新的 self.dest（步骤 20 缺陷修复：
    /// 切换盘符后剩余空间与校验必须随所选驱动器动态变化，NSIS
    /// DirectoryPage 语义）；输入为空时回退默认目录。
    fn current_dest(&self) -> PathBuf {
        miedit_core::installdir::parse_dir_text(&self.dir_text)
            .map(PathBuf::from)
            .unwrap_or_else(|| self.dest.clone())
    }

    /// 目录页下一页/安装可用性（NSIS DirectoryPage 校验语义）：
    /// 目录无效（空/相对路径/非法字符/盘不存在）或根目录未允许 → 禁用；
    /// 空间不足 → 禁用；空间未知 → 放行（保持旧行为）。
    fn dir_next_enabled(&self) -> bool {
        if self.dir_issue().is_some() {
            return false;
        }
        match self.disk_free() {
            Some(free) => free >= self.selected_bytes(),
            None => true,
        }
    }

    /// 安装目录校验（纯字符串判定 + 盘存在性补充检查）。
    /// 返回 None 表示通过；Some(issue) 时 UI 显示对应提示并禁用前进。
    fn dir_issue(&self) -> Option<miedit_core::installdir::DirIssue> {
        let allow = self.pkg.config.install.allow_root_dir_install;
        if let Err(issue) = miedit_core::installdir::classify_inst_dir(&self.dir_text, allow) {
            return Some(issue);
        }
        // 盘/共享必须真实存在：不存在的盘符查不到可用空间，也不应放行
        let path = self.current_dest();
        match miedit_core::installdir::drive_root_of(&path.to_string_lossy()) {
            Some(root) if PathBuf::from(&root).exists() => None,
            _ => Some(miedit_core::installdir::DirIssue::Invalid),
        }
    }

    fn disk_free(&self) -> Option<u64> {
        miedit_core::winutil::disk_free_space(&self.current_dest())
    }

    fn check_app_running(&self) -> bool {
        if self.pkg.config.app.exe.is_empty() {
            return false;
        }
        let exe = self.dest.join(&self.pkg.config.app.exe);
        miedit_core::winutil::is_file_locked(&exe)
    }

    fn start_install(&mut self, ctx: &egui::Context) {
        self.install_finished = false;
        // 步骤 20：目录页输入清洗（去空白/包裹引号/裸盘符规范化）与显示口径统一
        let dest = self.current_dest();
        self.dest = dest.clone();

        // 主程序运行中检测（NSIS/Inno 常见预检）
        if self.check_app_running() {
            self.page = Page::AppRunning;
            return;
        }

        let rules = self.selected_rules();
        // 步骤 30：传递后台 CRC 校验门（安装线程在提取前等待校验结果）
        let gate = Some(self.crc_gate.clone());
        self.state = install::spawn(
            self.pkg.clone(),
            dest,
            Some(ctx.clone()),
            rules,
            self.language,
            self.no_shortcuts,
            gate,
        );
        self.page = Page::Install;
    }

    /// 修复模式（路线图步骤 17）：全量恢复组件选择后走常规安装流程
    /// （重解压全部文件 + 重建快捷方式/注册表，与 Inno 修复语义一致）。
    fn start_repair(&mut self, ctx: &egui::Context) {
        self.component_checked = miedit_core::repair::repair_selection(self.pkg.config.components.len());
        self.install_type = self.pkg.config.install_types.len(); // Custom（全选态）
        self.start_install(ctx);
    }

    fn next(&mut self, ctx: &egui::Context) {
        match self.page {
            Page::Language => self.page = Page::Welcome,
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
                // 步骤 18（NSIS INSTFILES 语义）：完成后停留在安装详情页，
                // 显示完成状态与全量日志，用户点「下一步」进入完成页
                self.install_finished = true;
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

    /// 步骤 8：把用户在文件错误模态中的选择写给后台安装线程。
    fn answer_prompt(&self, choice: miedit_core::fileerror::FileErrorChoice, ctx: &egui::Context) {
        if let Ok(mut s) = self.state.lock() {
            s.prompt_channel.answer(choice);
        }
        ctx.request_repaint();
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
            &txt(cfg, self.language, "run_app"),
            &cfg.app.name,
            &cfg.app.version,
        );
        vec![(name, exe, String::new())]
    }

    // ---- 主操作（NSIS：Enter 触发默认按钮 / Esc 取消）----

    fn primary_action(&mut self, ctx: &egui::Context) {
        match self.page {
            Page::Language | Page::Welcome | Page::License | Page::Components | Page::Dir => {
                let enabled = match self.page {
                    Page::License => self.license_accepted,
                    Page::Dir => self.dir_next_enabled(),
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
            Page::RepairConfirm => self.start_repair(ctx),
            // 步骤 18：详情停留页完成后「下一步」→ 完成页（NSIS INSTFILES → FINISH）
            Page::Install => {
                if self.install_finished {
                    self.page = Page::Finish;
                }
            }
            Page::Error => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    // ---- 卡片内容 ----

    fn content(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        let cfg = &self.pkg.config;
        match self.page {
            // 步骤 15：语言选择页（NSIS MUI 语义：欢迎页前，选中后文案立即切换）
            Page::Language => {
                ui.heading(
                    egui::RichText::new(txt(cfg, self.language, "language_title"))
                        .color(self.theme.text)
                        .size(28.0)
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(txt(cfg, self.language, "language_desc"))
                        .color(self.theme.text_dim)
                        .size(14.0),
                );
                ui.add_space(10.0);
                let mut lang_next = self.language;
                egui::Frame::none()
                    .fill(self.theme.field)
                    .rounding(10.0)
                    .stroke(Stroke::new(1.0_f32, self.theme.card_stroke))
                    .inner_margin(egui::Margin::same(14.0))
                    .show(ui, |ui| {
                        for (i, l) in cfg.ui.languages.iter().enumerate() {
                            ui.radio_value(
                                &mut lang_next,
                                Some(i),
                                egui::RichText::new(&l.name)
                                    .color(self.theme.text)
                                    .size(15.0),
                            );
                        }
                    });
                self.language = lang_next;
            }
            // 步骤 17：修复确认页（ARP ModifyPath 入口，Inno 修复向导语义）
            Page::RepairConfirm => {
                ui.add_space(6.0);
                draw_badge(ui, Color32::from_rgb(0x4C, 0xA8, 0xF0), "R", 34.0);
                ui.add_space(10.0);
                ui.heading(
                    egui::RichText::new(txt(cfg, self.language, "repair_title"))
                        .color(self.theme.text)
                        .size(30.0)
                        .strong(),
                );
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(
                        miedit_core::config::fill_template(
                            &txt(cfg, self.language, "repair_body"),
                            &cfg.app.name,
                            &cfg.app.version,
                        )
                        .replace("{dir}", &self.dest_display()),
                    )
                    .color(self.theme.text_dim)
                    .size(15.0),
                );
                ui.add_space(10.0);
                let s = miedit_core::repair::summarize(cfg);
                ui.label(
                    egui::RichText::new(
                        txt(cfg, self.language, "repair_summary")
                            .replace("{files}", &s.file_rules.to_string())
                            .replace("{shortcuts}", &s.shortcuts.to_string())
                            .replace("{registry}", &s.registry_values.to_string()),
                    )
                    .color(self.theme.text)
                    .size(13.5)
                    .strong(),
                );
            }
            Page::Welcome => {
                let chip = txt(cfg, self.language, "welcome_chip");
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
                    egui::RichText::new(
                        txt(cfg, self.language, "version_line")
                            .replace("{version}", &cfg.app.version)
                            .replace(
                                "{publisher}",
                                if cfg.app.publisher.is_empty() { "—" } else { &cfg.app.publisher },
                            ),
                    )
                    .color(self.theme.text_dim)
                    .size(15.0),
                );
                ui.add_space(16.0);
                let body = miedit_core::config::fill_template(
                    &txt(cfg, self.language, "welcome_body"),
                    &cfg.app.name,
                    &cfg.app.version,
                );
                ui.label(egui::RichText::new(body).color(self.theme.text_dim).size(14.5));
            }
            Page::License => {
                ui.heading(
                    egui::RichText::new(txt(cfg, self.language, "license_title"))
                        .color(self.theme.text)
                        .size(28.0)
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(txt(cfg, self.language, "license_desc"))
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
                    .stroke(Stroke::new(1.0_f32, self.theme.card_stroke))
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
                        egui::RichText::new(txt(cfg, self.language, "license_accept"))
                            .color(if can_accept { self.theme.text } else { self.theme.text_dim })
                            .size(14.0),
                    );
                });
                if scroll_required && !self.license_scrolled {
                    ui.label(
                        egui::RichText::new(txt(cfg, self.language, "license_scroll_hint"))
                            .color(self.theme.text_dim)
                            .size(12.0),
                    );
                }
            }
            Page::Components => {
                ui.heading(
                    egui::RichText::new(txt(cfg, self.language, "components_title"))
                        .color(self.theme.text)
                        .size(28.0)
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(miedit_core::config::fill_template(
                        &txt(cfg, self.language, "components_desc"),
                        &cfg.app.name,
                        &cfg.app.version,
                    ))
                    .color(self.theme.text_dim)
                    .size(14.0),
                );
                ui.add_space(8.0);
                let mut checked_next = self.component_checked.clone();
                let mut desc_next = self.desc_index;
                // 步骤 10：InstallTypes 类型单选行（NSIS 组件页顶部类型语义）：
                // 选中类型 → 组件勾选重置为该类型定义；末尾附带 Custom = 当前手动状态。
                let types = &cfg.install_types;
                let custom_index = types.len();
                if !types.is_empty() {
                    let mut type_next = self.install_type.min(custom_index);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(txt(cfg, self.language, "components_type_title"))
                            .color(self.theme.text_dim)
                            .size(13.0),
                        );
                        for (i, t) in types.iter().enumerate() {
                            ui.radio_value(
                                &mut type_next,
                                i,
                                egui::RichText::new(&t.name)
                                    .color(self.theme.text)
                                    .size(13.5),
                            );
                        }
                        ui.radio_value(
                            &mut type_next,
                            custom_index,
                            egui::RichText::new(txt(cfg, self.language, "components_type_custom"))
                            .color(self.theme.text)
                            .size(13.5),
                        );
                    });
                    if type_next != self.install_type {
                        if let Some(sel) =
                            miedit_core::installtypes::selection_for_type(types, &cfg.components, type_next)
                        {
                            checked_next = sel;
                        }
                    }
                }
                ui.horizontal(|ui| {
                    if ui
                        .add(theme::secondary_button(
                            &txt(cfg, self.language, "components_select_all"),
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
                            &txt(cfg, self.language, "components_reset"),
                            &self.theme,
                        ))
                        .clicked()
                    {
                        // 有安装类型时重置 = 回到默认类型；否则回退到仅必装
                        match miedit_core::installtypes::default_type_index(types) {
                            Some(i) => {
                                if let Some(sel) = miedit_core::installtypes::selection_for_type(
                                    types,
                                    &cfg.components,
                                    i,
                                ) {
                                    checked_next = sel;
                                }
                            }
                            None => {
                                for (i, c) in cfg.components.iter().enumerate() {
                                    if let Some(v) = checked_next.get_mut(i) {
                                        *v = c.required;
                                    }
                                }
                            }
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // 步骤 21：NSIS components page「Space required」联动——
                        // 随组件勾选实时统计并显示带标签的空间需求
                        ui.label(
                            egui::RichText::new(
                                txt(cfg, self.language, "disk_space").replace(
                                    "{size}",
                                    &miedit_core::util::human_size(self.selected_bytes()),
                                ),
                            )
                            .color(self.theme.text_dim)
                            .size(12.5),
                        );
                    });
                });
                ui.add_space(4.0);
                egui::Frame::none()
                    .fill(self.theme.field)
                    .rounding(10.0)
                    .stroke(Stroke::new(1.0_f32, self.theme.card_stroke))
                    .inner_margin(egui::Margin::same(14.0))
                    .show(ui, |ui| {
                        // 步骤 24：组件分组树（NSIS SectionGroup/SectionGroupEnd 语义）——
                        // `group = "父组/子组"` 路径 → 缩进树行；组行可折叠/展开，
                        // 组复选框三态联动子组件（必装组件保持勾选）；
                        // 安装类型与 CLI --components= 仍按扁平索引运作，互不影响。
                        let rows = miedit_core::componenttree::build(&cfg.components);
                        let mut collapsed_next = self.comp_collapsed.clone();
                        for row in &rows {
                            // 折叠判定：组行仅在「真祖先」被折叠时隐藏（自身折叠仍渲染，
                            // 以便展开）；组件行在整条组路径任一层被折叠时隐藏
                            let skip = match row.kind {
                                miedit_core::componenttree::RowKind::Group => {
                                    let parent = row
                                        .group_path
                                        .rsplit_once('/')
                                        .map(|(p, _)| p.to_string())
                                        .unwrap_or_default();
                                    miedit_core::componenttree::is_hidden(&collapsed_next, &parent)
                                }
                                miedit_core::componenttree::RowKind::Component => {
                                    miedit_core::componenttree::is_hidden(
                                        &collapsed_next,
                                        &row.group_path,
                                    )
                                }
                            };
                            if skip {
                                continue;
                            }
                            match row.kind {
                                miedit_core::componenttree::RowKind::Group => {
                                    let state = miedit_core::componenttree::group_state(
                                        &cfg.components,
                                        &checked_next,
                                        &row.group_path,
                                    );
                                    ui.horizontal(|ui| {
                                        ui.add_space(row.depth as f32 * 18.0);
                                        let is_collapsed =
                                            collapsed_next.contains(&row.group_path);
                                        let arrow = if is_collapsed { "▸" } else { "▾" };
                                        if ui
                                            .add(
                                                egui::Button::new(
                                                    egui::RichText::new(arrow)
                                                        .color(self.theme.text_dim)
                                                        .size(12.0),
                                                )
                                                .frame(false),
                                            )
                                            .clicked()
                                        {
                                            if is_collapsed {
                                                collapsed_next.remove(&row.group_path);
                                            } else {
                                                collapsed_next.insert(row.group_path.clone());
                                            }
                                        }
                                        let mut display =
                                            state != miedit_core::componenttree::GroupState::None_;
                                        let resp = ui.checkbox(
                                            &mut display,
                                            egui::RichText::new(&row.label)
                                                .color(self.theme.text)
                                                .size(14.5)
                                                .strong(),
                                        );
                                        if resp.changed() {
                                            miedit_core::componenttree::toggle_group(
                                                &cfg.components,
                                                &mut checked_next,
                                                &row.group_path,
                                                display,
                                            );
                                        }
                                    });
                                    ui.add_space(2.0);
                                }
                                miedit_core::componenttree::RowKind::Component => {
                                    let i = match row.component {
                                        Some(i) => i,
                                        None => continue,
                                    };
                                    let comp = &cfg.components[i];
                                    let checked = checked_next.get(i).copied().unwrap_or(true);
                                    let mut next = checked;
                                    let row_r = ui.horizontal(|ui| {
                                        ui.add_space(row.depth as f32 * 18.0);
                                        let resp = ui
                                            .add_enabled_ui(!comp.required, |ui| {
                                                ui.checkbox(
                                                    &mut next,
                                                    egui::RichText::new(&row.label)
                                                        .color(self.theme.text)
                                                        .size(15.0)
                                                        .strong(),
                                                )
                                            })
                                            .inner;
                                        // 步骤 11：悬停/点击组件行 → 描述面板跟随（NSIS 描述框语义）
                                        if resp.hovered() || resp.clicked() {
                                            desc_next = Some(i);
                                        }
                                        if comp.required {
                                            chip_small(
                                                ui,
                                                txt(cfg, self.language, "components_required"),
                                                self.theme.accent,
                                            );
                                        }
                                    });
                                    // 整行作为悬停面（必装组件的复选框处于禁用态，其响应不含悬停）
                                    let hover = ui.interact(
                                        row_r.response.rect,
                                        egui::Id::new(("comp_row_hover", i)),
                                        egui::Sense::hover(),
                                    );
                                    if hover.hovered() {
                                        desc_next = Some(i);
                                    }
                                    checked_next[i] = if comp.required { true } else { next };
                                    ui.add_space(6.0);
                                }
                            }
                        }
                        self.comp_collapsed = collapsed_next;
                    });
                ui.add_space(6.0);
                // 步骤 11：组件描述面板（NSIS 组件页描述框语义；必装组件带「必需」徽标）
                let sel_comp = match desc_next {
                    Some(i) => cfg.components.get(i),
                    None => None,
                };
                let has_desc = sel_comp
                    .map(|c| !c.description.trim().is_empty())
                    .unwrap_or(false);
                egui::Frame::none()
                    .fill(self.theme.field)
                    .rounding(10.0)
                    .stroke(Stroke::new(1.0_f32, self.theme.card_stroke))
                    .inner_margin(egui::Margin::same(12.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(txt(cfg, self.language, "components_desc_panel"))
                                .color(self.theme.text_dim)
                                .size(12.0)
                                .strong(),
                            );
                            if let Some(c) = sel_comp {
                                if c.required {
                                    chip_small(
                                        ui,
                                        txt(cfg, self.language, "components_required"),
                                        self.theme.accent,
                                    );
                                }
                            }
                        });
                        let text = match sel_comp {
                            Some(c) if has_desc => c.description.clone(),
                            _ => txt(cfg, self.language, "components_desc_empty"),
                        };
                        ui.label(
                            egui::RichText::new(text)
                                .color(if has_desc { self.theme.text } else { self.theme.text_dim })
                                .size(13.5),
                        );
                    });
                self.component_checked = checked_next;
                self.desc_index = desc_next;
                // 步骤 10：当前勾选状态命中已定义类型则跟随之，否则自动切到 Custom（NSIS 语义）
                self.install_type = miedit_core::installtypes::matching_type(
                    &cfg.install_types,
                    &cfg.components,
                    &self.component_checked,
                )
                .unwrap_or_else(|| cfg.install_types.len());
            }
            Page::Dir => {
                ui.heading(
                    egui::RichText::new(txt(cfg, self.language, "dir_title"))
                        .color(self.theme.text)
                        .size(28.0)
                        .strong(),
                );
                ui.add_space(4.0);
                let desc = miedit_core::config::fill_template(
                    &txt(cfg, self.language, "dir_desc"),
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
                        .stroke(Stroke::new(1.0_f32, self.theme.card_stroke))
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
                            &txt(cfg, self.language, "browse"),
                            &self.theme,
                        ))
                        .clicked()
                    {
                        if let Some(dir) = rfd::FileDialog::new()
                            .set_title(txt(cfg, self.language, "dir_title"))
                            .pick_folder()
                        {
                            self.dir_text = dir.display().to_string();
                        }
                    }
                });

                ui.add_space(6.0);
                let need = self.selected_bytes();
                let space =
                    txt(cfg, self.language, "disk_space").replace("{size}", &miedit_core::util::human_size(need));
                ui.label(egui::RichText::new(space).color(self.theme.text_dim).size(13.0));
                // 步骤 20 缺陷修复：可用空间基于当前输入目录所在盘实时查询
                //（NSIS DirectoryPage：Space available 随所选驱动器刷新）
                if let Some(free) = self.disk_free() {
                    let free_s = txt(cfg, self.language, "free_space")
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
                // 步骤 20：目录校验提示（NSIS：无效路径/根目录拒绝 → 提示并禁用前进）
                if let Some(issue) = self.dir_issue() {
                    let key = miedit_core::installdir::dir_issue_key(issue);
                    ui.add_space(2.0);
                    ui.label(
                        egui::RichText::new(txt(cfg, self.language, key))
                            .color(Color32::from_rgb(0xE5, 0x48, 0x4D))
                            .size(12.5),
                    );
                }
                // Inno DirExistsWarning：目标目录已存在且非空时提示
                if let Some(parsed) = miedit_core::installdir::parse_dir_text(&self.dir_text) {
                    let d = PathBuf::from(&parsed);
                    if d.is_dir() {
                        let non_empty = std::fs::read_dir(&d)
                            .map(|mut it| it.next().is_some())
                            .unwrap_or(false);
                        if non_empty {
                            ui.add_space(2.0);
                            ui.label(
                                egui::RichText::new(txt(cfg, self.language, "dir_exists_warning"))
                                    .color(Color32::from_rgb(0xF5, 0xA5, 0x24))
                                    .size(12.5),
                            );
                        }
                    }
                }
            }
            Page::Install => {
                // 步骤 18：完成详情停留页——标题与状态行切换为完成态
                if self.install_finished {
                    ui.heading(
                        egui::RichText::new(txt(cfg, self.language, "finish_title"))
                            .color(self.theme.text)
                            .size(28.0)
                            .strong(),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(format!("√ {}", miedit_core::config::fill_template(
                            &txt(cfg, self.language, "finish_body"),
                            &cfg.app.name,
                            &cfg.app.version,
                        )))
                        .color(Color32::from_rgb(0x3D, 0xD6, 0x8C))
                        .size(14.0)
                        .strong(),
                    );
                } else {
                ui.heading(
                    egui::RichText::new(miedit_core::config::fill_template(
                        &txt(cfg, self.language, "installing"),
                        &cfg.app.name,
                        &cfg.app.version,
                    ))
                    .color(self.theme.text)
                    .size(28.0)
                    .strong(),
                );
                }
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
                                    txt(cfg, self.language, "preparing")
                                }
                                _ => txt(cfg, self.language, "finalizing"),
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
                            txt(cfg, self.language, "extracting").replace("{file}", &cur),
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
                        egui::RichText::new(
                            txt(cfg, self.language, "progress_stats")
                                .replace("{done}", &done_f.to_string())
                                .replace("{total}", &total_f.to_string())
                                .replace("{bytes}", &miedit_core::util::human_size(done_b))
                                .replace("{speed}", &speed_s),
                        )
                        .color(self.theme.text_dim)
                        .size(12.5),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = if self.details_open {
                            txt(cfg, self.language, "details_hide")
                        } else {
                            txt(cfg, self.language, "details_show")
                        };
                        if ui
                            .add(theme::secondary_button(&label, &self.theme))
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
                        .stroke(Stroke::new(1.0_f32, self.theme.card_stroke))
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
                // 修复模式完成页文案切换（步骤 17：修复完成 ≠ 安装完成）
                let finish_title_key = if self.repair { "repair_finish_title" } else { "finish_title" };
                let finish_body_key = if self.repair { "repair_finish_body" } else { "finish_body" };
                ui.heading(
                    egui::RichText::new(txt(cfg, self.language, finish_title_key))
                        .color(self.theme.text)
                        .size(34.0)
                        .strong(),
                );
                ui.add_space(4.0);
                let body = miedit_core::config::fill_template(
                    &txt(cfg, self.language, finish_body_key),
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
                // 步骤 9：有文件被计划重启替换时的琥珀提示（NSIS SetRebootFlag 完成页语义）
                let pending_reboot = self
                    .state
                    .lock()
                    .ok()
                    .map(|s| s.pending_reboot.clone())
                    .unwrap_or_default();
                if !pending_reboot.is_empty() {
                    ui.add_space(8.0);
                    let amber = Color32::from_rgb(0xF5, 0xA5, 0x24);
                    egui::Frame::none()
                        .fill(amber.linear_multiply(0.12))
                        .rounding(10.0)
                        .stroke(Stroke::new(1.0_f32, amber.linear_multiply(0.5)))
                        .inner_margin(egui::Margin::same(10.0))
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.label(
                                egui::RichText::new(
                                    txt(cfg, self.language, "finish_reboot_required")
                                        .replace("{count}", &pending_reboot.len().to_string()),
                                )
                                .color(amber)
                                .size(13.5)
                                .strong(),
                            );
                            for p in pending_reboot.iter().take(4) {
                                ui.label(
                                    egui::RichText::new(format!("· {p}"))
                                        .color(self.theme.text_dim)
                                        .size(12.0)
                                        .monospace(),
                                );
                            }
                            if pending_reboot.len() > 4 {
                                ui.label(
                                    egui::RichText::new(
                                        txt(cfg, self.language, "finish_reboot_more")
                                            .replace("{count}", &pending_reboot.len().to_string()),
                                    )
                                    .color(self.theme.text_dim)
                                    .size(12.0),
                                );
                            }
                        });
                }
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
                    egui::RichText::new(txt(cfg, self.language, "app_running_title"))
                        .color(self.theme.text)
                        .size(30.0)
                        .strong(),
                );
                ui.add_space(6.0);
                let body = miedit_core::config::fill_template(
                    &txt(cfg, self.language, "app_running_body"),
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
                    egui::RichText::new(txt(cfg, self.language, "error_title"))
                        .color(self.theme.text)
                        .size(30.0)
                        .strong(),
                );
                ui.add_space(6.0);
                egui::Frame::none()
                    .fill(self.theme.field)
                    .rounding(10.0)
                    .stroke(Stroke::new(1.0_f32, self.theme.card_stroke))
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
        let t_next = txt(&cfg, self.language, "next");
        let t_back = txt(&cfg, self.language, "back");
        let t_cancel = txt(&cfg, self.language, "cancel");
        let t_install = txt(&cfg, self.language, "install_now");
        let t_done = txt(&cfg, self.language, "done_btn");
        let t_retry = txt(&cfg, self.language, "retry");
        let t_continue = txt(&cfg, self.language, "continue_anyway");

        ui.horizontal(|ui| {
            match self.page {
                Page::Language | Page::Welcome | Page::License | Page::Components | Page::Dir => {
                    if ui.add(theme::secondary_button(&t_cancel, &self.theme)).clicked() {
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                    }
                    ui.add_space(4.0);
                    // 语言页为首页，不显示返回（NSIS 语言选择不可回退）
                    if !matches!(self.page, Page::Welcome | Page::Language) {
                        if ui.add(theme::secondary_button(&t_back, &self.theme)).clicked() {
                            self.back();
                        }
                        ui.add_space(4.0);
                    }
                    let enabled = match self.page {
                        Page::License => self.license_accepted,
                        Page::Dir => self.dir_next_enabled(),
                        _ => true,
                    };
                    let is_last = match self.page {
                        Page::Dir => true,
                        Page::License => !self.has_components() && !self.has_dir_page(),
                        Page::Components => !self.has_dir_page(),
                        Page::Welcome => !self.has_license() && !self.has_components() && !self.has_dir_page(),
                        Page::Language => false,
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
                // 步骤 18：详情停留页完成后显示「下一步」（NSIS INSTFILES → FINISH）
                Page::Install => {
                    if self.install_finished {
                        let t_next2 = txt(&cfg, self.language, "next");
                        if ui.add(theme::primary_button(&t_next2, &self.theme)).clicked() {
                            self.primary_action(ctx);
                        }
                    }
                }
                Page::RepairConfirm => {
                    let t_repair = txt(&cfg, self.language, "repair_now");
                    if ui.add(theme::secondary_button(&t_cancel, &self.theme)).clicked() {
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                    }
                    ui.add_space(4.0);
                    if ui.add(theme::primary_button(&t_repair, &self.theme)).clicked() {
                        self.primary_action(ctx);
                    }
                }
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
                        self.state = install::spawn(
                            self.pkg.clone(),
                            dest,
                            Some(ctx.clone()),
                            rules,
                            self.language,
                            self.no_shortcuts,
                            Some(self.crc_gate.clone()),
                        );
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

/// 文案解析（步骤 15 多语言）：`ui.text` 覆盖 > 语言包 `text` > 内置包 > 英文默认。
fn txt(cfg: &miedit_core::config::Config, lang: Option<usize>, key: &str) -> String {
    miedit_core::config::resolved_text(cfg, lang, key)
}

fn chip_small(ui: &mut egui::Ui, text: String, accent: Color32) {
    egui::Frame::none()
        .fill(accent.linear_multiply(0.16))
        .stroke(Stroke::new(1.0_f32, accent.linear_multiply(0.5)))
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
        .stroke(Stroke::new(1.0_f32, accent.linear_multiply(0.55)));
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
        if enter && (self.page != Page::Install || self.install_finished) {
            self.primary_action(ctx);
        }
        // 步骤 15：语言切换后同步窗口标题（NSIS 标题栏跟随语言）
        let new_title = {
            let cfg = &self.pkg.config;
            miedit_core::config::fill_template(
                &txt(cfg, self.language, "window_title"),
                &cfg.app.name,
                &cfg.app.version,
            )
        };
        if new_title != self.title {
            self.title = new_title.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(new_title));
        }
        self.poll_state();
        // 步骤 30：后台 CRC 失败 → 立即转错误页（不等用户点安装才发现）；
        // 壁纸解码完成 → 上传纹理（渐进式呈现：先渐变，解码完自动换壁纸）
        if self.page != Page::Error {
            if let Some(msg) = self.crc_gate.failed_msg() {
                self.page = Page::Error;
                self.err_msg = msg;
            }
        }
        if self.bg_tex.is_none() {
            if let Some(img) = self.bg_shared.lock().unwrap().take() {
                self.bg_tex = Some(ctx.load_texture("bg", img, egui::TextureOptions::LINEAR));
                ctx.request_repaint();
            }
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                let sr = ui.max_rect();
                theme::paint_background(
                    ui.painter(),
                    sr,
                    &self.theme,
                    self.bg_tex.as_ref(),
                    &self.pkg.config.ui.background_style,
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
                ui.painter().line_segment([c - s1, c + s1], Stroke::new(1.6_f32, self.theme.text_dim));
                ui.painter().line_segment([c - s2, c + s2], Stroke::new(1.6_f32, self.theme.text_dim));
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
                    Stroke::new(1.6_f32, self.theme.text_dim),
                );
                if mrect.clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
                }

                // 半透明卡片（液态玻璃：半透明浅色 + 高光描边，背景透出）
                let card = Rect::from_min_max(
                    Pos2::new(sr.left() + 44.0, sr.top() + 108.0),
                    Pos2::new(sr.right() - 44.0, sr.bottom() - 104.0),
                );
                ui.painter().rect_filled(card, 18.0, self.theme.card_fill());
                ui.painter().rect(
                    card,
                    18.0,
                    Color32::TRANSPARENT,
                    Stroke::new(1.0_f32, self.theme.stroke_color()),
                );
                // 步骤导航栏（HM NIS Edit 向导式，路线图步骤 21）：
                // 左列步骤列表 + 当前步高亮 + 已完成勾点；仅指示不点击
                // （NSIS 页面流线性，不可跳跃）
                let rail_w = if self.show_step_rail() && self.rail_current().is_some() {
                    150.0
                } else {
                    0.0
                };
                if rail_w > 0.0 {
                    let rail_rect = Rect::from_min_max(
                        card.min,
                        Pos2::new(card.left() + rail_w, card.bottom()),
                    );
                    self.paint_step_rail(ui, rail_rect);
                }
                let content_left = card.left() + rail_w + if rail_w > 0.0 { 14.0 } else { 28.0 };
                let content = Rect::from_min_max(
                    Pos2::new(content_left, card.top() + 22.0),
                    Pos2::new(card.right() - 28.0, card.bottom() - 22.0),
                );
                ui.allocate_ui_at_rect(content, |ui| {
                    self.content(ui, ctx);
                });
            });

        // 底部按钮（浮动）
        egui::Area::new(egui::Id::new("buttons"))
            .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-44.0, -32.0))
            .show(ctx, |ui| {
                self.buttons(ui, ctx);
            });

        // 步骤 8：文件写入错误模态（NSIS 文件错误对话框：重试/忽略/中止）。
        // 后台线程打开请求后在此渲染遮罩 + 卡片，答复写入 PromptChannel。
        let prompt = self.state.lock().ok().and_then(|s| {
            if s.prompt_channel.pending() {
                s.prompt_channel
                    .current()
                    .map(|p| (p.path.clone(), p.error.clone()))
            } else {
                None
            }
        });
        if let Some((path, error)) = prompt {
            let cfg = self.pkg.config.clone();
            // 遮罩：压暗背景并吞掉模态外的点击（NSIS 模态语义）
            egui::Area::new(egui::Id::new("file-error-scrim"))
                .order(egui::Order::Foreground)
                .anchor(Align2::LEFT_TOP, Vec2::ZERO)
                .show(ctx, |ui| {
                    let size = ctx.screen_rect().size();
                    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
                    ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha(150));
                    let _ = resp;
                });
            egui::Area::new(egui::Id::new("file-error-modal"))
                .order(egui::Order::Foreground)
                .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    let w = 480.0_f32.min(ctx.screen_rect().width() - 80.0).max(320.0);
                    egui::Frame::none()
                        .fill(self.theme.card)
                        .rounding(14.0)
                        .stroke(Stroke::new(1.0_f32, self.theme.card_stroke))
                        .inner_margin(egui::Margin::same(20.0))
                        .show(ui, |ui| {
                            ui.set_min_width(w);
                            draw_badge(ui, Color32::from_rgb(0xF5, 0xA5, 0x24), "!", 30.0);
                            ui.add_space(6.0);
                            ui.heading(
                                egui::RichText::new(txt(&cfg, self.language, "file_error_title"))
                                    .color(self.theme.text)
                                    .size(22.0)
                                    .strong(),
                            );
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(
                                    txt(&cfg, self.language, "file_error_body")
                                        .replace("{file}", &path)
                                        .replace("{error}", &error),
                                )
                                .color(self.theme.text_dim)
                                .size(13.0),
                            );
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(txt(&cfg, self.language, "file_error_hint"))
                                    .color(self.theme.text_dim)
                                    .size(12.5),
                            );
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                if ui
                                    .add(theme::secondary_button(
                                        &txt(&cfg, self.language, "file_error_abort"),
                                        &self.theme,
                                    ))
                                    .clicked()
                                {
                                    self.answer_prompt(miedit_core::fileerror::FileErrorChoice::Abort, ctx);
                                }
                                if ui
                                    .add(theme::secondary_button(
                                        &txt(&cfg, self.language, "file_error_ignore"),
                                        &self.theme,
                                    ))
                                    .clicked()
                                {
                                    self.answer_prompt(miedit_core::fileerror::FileErrorChoice::Ignore, ctx);
                                }
                                if ui
                                    .add(theme::primary_button(
                                        &txt(&cfg, self.language, "retry"),
                                        &self.theme,
                                    ))
                                    .clicked()
                                {
                                    self.answer_prompt(miedit_core::fileerror::FileErrorChoice::Retry, ctx);
                                }
                            });
                        });
                });
        }
    }
}
