//! MIEdit 安装包构建向导 —— HM NIS Edit / Inno Setup 式分步向导。
//!
//! 9 步：欢迎 → 应用程序信息 → 安装程序选项 → 目录与授权 → 文件与组件 →
//! 快捷方式 → 安装完成后运行 → 卸载与注册表 → 构建。
//! 每页：标题 + 说明 + 页脚 ⟨ 上一步(B) | 下一步(N) | 取消 ⟩；项目脚本与 CLI 互通。

use crate::{
    accent_color, default_config, preview, ACCENT_PRESETS, AMBER, BG_BOTTOM, BG_TOP, CARD,
    CARD_STROKE, FIELD, GREEN, RED, TEXT, TEXT_DEFAULTS_ZH, TEXT_DIM, WINDOW_HINT,
};
use crate::scriptide::{self, IdeAction};
use anyhow::Context as _;
use eframe::egui::{self, Color32, FontId, Pos2, Rect, Sense, TextureHandle, Vec2};
use miedit_compiler::{build_package, locate_components, BuildOptions, BuildSummary};
use miedit_core::config::Config;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

// ---------------------------------------------------------------------------
// 字体与样式
// ---------------------------------------------------------------------------

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let candidates = [
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\msyhl.ttc",
        "C:\\Windows\\Fonts\\simhei.ttf",
        "C:\\Windows\\Fonts\\simsun.ttc",
    ];
    for (i, p) in candidates.iter().enumerate() {
        if let Ok(data) = std::fs::read(p) {
            let name = format!("cjk{i}");
            fonts.font_data.insert(name.clone(), egui::FontData::from_owned(data));
            fonts.families.entry(egui::FontFamily::Proportional).or_default().push(name.clone());
            fonts.families.entry(egui::FontFamily::Monospace).or_default().push(name);
            break;
        }
    }
    ctx.set_fonts(fonts);
}

pub fn apply_style(ctx: &egui::Context) {
    ctx.set_visuals(egui::Visuals::dark());
    ctx.style_mut(|st| {
        let v = &mut st.visuals;
        v.panel_fill = BG_TOP;
        v.window_fill = CARD;
        v.extreme_bg_color = FIELD;
        v.faint_bg_color = Color32::from_rgb(31, 34, 43);
        v.selection.bg_fill = Color32::from_rgb(0x4F, 0x8C, 0xFF);
        v.widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
        v.widgets.inactive.bg_fill = Color32::from_rgb(32, 36, 46);
        v.widgets.inactive.bg_stroke = egui::Stroke::NONE;
        v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, TEXT);
        v.widgets.hovered.bg_fill = Color32::from_rgb(42, 47, 60);
        v.widgets.active.bg_fill = Color32::from_rgb(0x4F, 0x8C, 0xFF);
        st.spacing.item_spacing = Vec2::new(8.0, 8.0);
        st.spacing.button_padding = Vec2::new(14.0, 6.0);
        st.spacing.scroll = egui::style::ScrollStyle::thin();
    });
}

// ---------------------------------------------------------------------------
// 状态
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
enum BuildState {
    #[default]
    Idle,
    Running,
    Done(Box<BuildSummary>, Vec<String>),
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Step {
    Welcome = 0,
    AppInfo = 1,
    Options = 2,
    DirLicense = 3,
    FilesComponents = 4,
    Shortcuts = 5,
    RunAfter = 6,
    UninstallReg = 7,
    Script = 8,
    Build = 9,
}

const STEP_TITLES: [(&str, &str); 10] = [
    ("欢迎使用 MIEdit 安装包构建向导", ""),
    ("应用程序信息", "请指定关于应用程序的基本信息。"),
    ("安装程序选项", "请指定安装程序的外观与输出选项。"),
    ("应用程序的目录与授权信息", "请指定应用程序的默认目录与许可协议。"),
    ("应用程序文件与组件", "请指定要打包的文件与安装组件。"),
    ("应用程序快捷方式", "请指定为应用程序创建哪些快捷方式。"),
    ("安装完成后运行", "请指定安装程序完成时要运行的任务。"),
    ("卸载程序与注册表", "请指定卸载相关属性与注册表写入项。"),
    ("MI 脚本（.mi）", "像 VSCode 一样直接编辑 MI 脚本（自研轻量脚本，TOML 语法）。"),
    ("构建安装程序", "确认配置并生成自解压安装程序。"),
];

/// 左栏导航短名（HM NIS Edit 步骤树）
const STEP_SHORT: [&str; 10] = [
    "欢迎",
    "应用信息",
    "安装选项",
    "目录与许可",
    "文件与组件",
    "快捷方式",
    "完成后运行",
    "卸载与注册表",
    "MI 脚本",
    "构建",
];

/// 左栏当前步底色（accent 低透明近似值，深色主题）
const RAIL_CURRENT_BG: Color32 = Color32::from_rgb(45, 58, 96);

struct RegRow {
    key: String,
    values: String,
}

pub struct BuilderApp {
    cfg: Config,
    step: Step,
    project_file: Option<PathBuf>,
    output: String,
    build: Arc<Mutex<BuildState>>,
    build_log: Arc<Mutex<Vec<String>>>,
    bg_tex: Option<TextureHandle>,
    license_preview: Option<String>,
    reg_rows: Vec<RegRow>,
    flash: Option<(String, Instant)>,
    files_stat: (usize, u64),
    /// 第 9 步：脚本编辑缓冲与脏标记
    script_text: String,
    script_dirty: bool,
    /// VSCode 式 IDE 状态（查找/面板/转到行/换行开关，路线图步骤 32）
    ide: scriptide::IdeState,
    /// 左栏导航待跳转步骤（egui 借用规则：rail 内记录，update 末尾执行）
    pending_jump: Option<usize>,
}

impl BuilderApp {
    pub fn new(project: Option<PathBuf>, ctx: &egui::Context) -> Self {
        let cfg = default_config();
        let mut app = Self {
            cfg,
            step: Step::Welcome,
            project_file: None,
            output: String::new(),
            build: Arc::new(Mutex::new(BuildState::Idle)),
            build_log: Arc::new(Mutex::new(Vec::new())),
            bg_tex: None,
            license_preview: None,
            reg_rows: vec![],
            flash: None,
            files_stat: (0, 0),
            script_text: String::new(),
            script_dirty: false,
            ide: scriptide::IdeState::default(),
            pending_jump: None,
        };
        if let Some(p) = project {
            if let Err(e) = app.open_project(&p, ctx) {
                app.set_flash(format!("打开项目失败: {e:#}"));
            } else {
                app.step = Step::AppInfo; // 打开项目后直接进入第二步
            }
        }
        app.refresh_files_stat();
        app
    }

    fn set_flash(&mut self, msg: impl Into<String>) {
        self.flash = Some((msg.into(), Instant::now()));
    }

    fn accent(&self) -> Color32 {
        accent_color(&self.cfg)
    }

    fn step_index(&self) -> usize {
        self.step as usize
    }

    fn output_path(&self) -> PathBuf {
        if self.output.trim().is_empty() {
            let slug = miedit_core::util::slugify(
                if self.cfg.app.name.trim().is_empty() {
                    "app"
                } else {
                    &self.cfg.app.name
                },
            );
            let dir = self
                .project_file
                .as_ref()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            dir.join(format!("{}-{}-setup.exe", slug, self.cfg.app.version))
        } else {
            PathBuf::from(self.output.trim())
        }
    }

    fn script_dir(&self) -> PathBuf {
        self.project_file
            .as_ref()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
    }

    fn refresh_files_stat(&mut self) {
        let base = self.script_dir();
        let mut count = 0usize;
        let mut total = 0u64;
        for spec in &self.cfg.files {
            if let Ok(list) = miedit_core::files::gather(spec, &base) {
                for f in list {
                    if !f.is_dir {
                        count += 1;
                        total += f.size;
                    }
                }
            }
        }
        self.files_stat = (count, total);
    }

    fn load_background(&mut self, ctx: &egui::Context) {
        self.bg_tex = None;
        if let Some(bp) = &self.cfg.ui.background {
            let p = self.script_dir().join(bp);
            if let Ok(dyn_img) = image::open(&p) {
                let rgba = dyn_img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                self.bg_tex = Some(ctx.load_texture("builder-bg", color, egui::TextureOptions::LINEAR));
            }
        }
    }

    fn load_license_preview(&mut self) {
        self.license_preview = None;
        if let Some(lp) = &self.cfg.ui.license {
            let p = self.script_dir().join(lp);
            self.license_preview = std::fs::read_to_string(&p).ok();
        }
    }

    fn sync_reg_from_cfg(&mut self) {
        self.reg_rows = self
            .cfg
            .registry
            .iter()
            .map(|r| RegRow {
                key: r.key.clone(),
                values: r
                    .values
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            })
            .collect();
    }

    fn apply_reg_to_cfg(&mut self) {
        self.cfg.registry = self
            .reg_rows
            .iter()
            .filter(|r| !r.key.trim().is_empty())
            .map(|r| {
                let mut values = std::collections::BTreeMap::new();
                for line in r.values.lines() {
                    if let Some((k, v)) = line.split_once('=') {
                        let k = k.trim();
                        if !k.is_empty() {
                            values.insert(k.to_string(), v.trim().to_string());
                        }
                    }
                }
                miedit_core::config::RegistrySpec {
                    key: r.key.trim().to_string(),
                    values,
                    ..Default::default()
                }
            })
            .collect();
    }

    // -----------------------------------------------------------------------
    // 项目打开 / 保存
    // -----------------------------------------------------------------------

    fn open_project(&mut self, path: &Path, ctx: &egui::Context) -> anyhow::Result<()> {
        let text = std::fs::read_to_string(path).with_context(|| path.display().to_string())?;
        let cfg: Config = toml::from_str(&text)
            .with_context(|| format!("解析失败: {}", path.display()))?;
        self.cfg = cfg;
        if self.cfg.ui.text.is_empty() {
            self.cfg.ui.text = TEXT_DEFAULTS_ZH
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
        }
        self.project_file = Some(path.to_path_buf());
        self.output.clear();
        self.load_background(ctx);
        self.load_license_preview();
        self.sync_reg_from_cfg();
        self.refresh_files_stat();
        self.set_flash(format!("已打开项目: {}", path.display()));
        Ok(())
    }

    fn save_project(&mut self) {
        self.apply_reg_to_cfg();
        self.cfg.files_map.clear();
        let target = match self.project_file.clone() {
            Some(p) => p,
            None => {
                let slug = miedit_core::util::slugify(
                    if self.cfg.app.name.trim().is_empty() {
                        "project"
                    } else {
                        &self.cfg.app.name
                    },
                );
                match rfd::FileDialog::new()
                    .set_file_name(format!("{slug}.mi"))
                    .add_filter("MI 脚本（.mi）", &["mi", "toml"])
                    .save_file()
                {
                    Some(p) => p,
                    None => return,
                }
            }
        };
        match toml::to_string_pretty(&self.cfg) {
            Ok(text) => match std::fs::write(&target, text) {
                Ok(()) => {
                    self.project_file = Some(target.clone());
                    self.set_flash(format!("已保存: {}", target.display()));
                }
                Err(e) => self.set_flash(format!("保存失败: {e}")),
            },
            Err(e) => self.set_flash(format!("序列化失败: {e}")),
        }
    }

    // -----------------------------------------------------------------------
    // 构建
    // -----------------------------------------------------------------------

    fn start_build(&mut self, ctx: &egui::Context) -> bool {
        if matches!(&*self.build.lock().unwrap(), BuildState::Running) {
            return false;
        }
        self.apply_reg_to_cfg();

        if self.cfg.app.name.trim().is_empty() {
            self.set_flash("应用程序名称为必填项（第 2 步）");
            return false;
        }
        if self.cfg.files.iter().all(|f| f.source.trim().is_empty()) {
            self.set_flash("请至少添加一个安装文件/目录（第 5 步）");
            return false;
        }

        let cfg = self.cfg.clone();
        let opts = BuildOptions {
            script_dir: self.script_dir(),
            output: self.output_path(),
            stub: None,
            uninstaller: None,
        };
        let build = self.build.clone();
        let log = self.build_log.clone();
        let ctx2 = ctx.clone();

        log.lock().unwrap().clear();
        *self.build.lock().unwrap() = BuildState::Running;

        std::thread::spawn(move || {
            let mut progress = |m: &str| {
                log.lock().unwrap().push(m.to_string());
                ctx2.request_repaint();
            };
            let result = build_package(&cfg, &opts, &mut progress);
            *build.lock().unwrap() = match result {
                Ok(s) => BuildState::Done(Box::new(s), log.lock().unwrap().clone()),
                Err(e) => BuildState::Failed(format!("{e:#}")),
            };
            ctx2.request_repaint();
        });
        true
    }
}

// ---------------------------------------------------------------------------
// 小部件
// ---------------------------------------------------------------------------

fn text_input(ui: &mut egui::Ui, text: &mut String, hint: &str, width: f32) -> egui::Response {
    let hint: egui::RichText =
        egui::RichText::new(hint.to_string()).color(Color32::from_rgb(96, 102, 116));
    let width = width.max(60.0);
    egui::Frame::none()
        .fill(FIELD)
        .rounding(8.0)
        .stroke(egui::Stroke::new(1.0_f32, CARD_STROKE))
        .inner_margin(egui::Margin::symmetric(10.0, 7.0))
        .show(ui, |ui| {
            ui.add_sized(
                [width, 18.0],
                egui::TextEdit::singleline(text)
                    .hint_text(hint)
                    .font(egui::TextStyle::Body)
                    .frame(false),
            )
        })
        .inner
}

/// “标签 + 输入框” 行（Inno 向导风格：左标签，右输入）。
fn field_row(ui: &mut egui::Ui, label: &str, text: &mut String, hint: &str, base: f32, input_reserve: f32) {
    ui.horizontal(|ui| {
        ui.add_sized([150.0, 0.0], egui::Label::new(egui::RichText::new(label).color(TEXT).size(13.5)));
        text_input(ui, text, hint, base - 150.0 - input_reserve);
    });
}

fn field_label(ui: &mut egui::Ui, label: &str) {
    ui.label(egui::RichText::new(label).color(TEXT_DIM).size(12.5));
}

fn primary_button<'a>(text: &'a str, accent: Color32) -> egui::Button<'a> {
    egui::Button::new(egui::RichText::new(text).color(Color32::WHITE).size(14.5).strong())
        .fill(accent)
        .rounding(9.0)
        .min_size(Vec2::new(0.0, 36.0))
}

fn nav_button<'a>(text: &'a str, accent: Option<Color32>) -> egui::Button<'a> {
    match accent {
        Some(c) => egui::Button::new(egui::RichText::new(text).color(Color32::WHITE).size(14.0).strong())
            .fill(c)
            .rounding(8.0)
            .min_size(Vec2::new(104.0, 34.0)),
        None => egui::Button::new(egui::RichText::new(text).color(TEXT).size(14.0))
            .fill(Color32::from_rgb(32, 36, 46))
            .stroke(egui::Stroke::new(1.0_f32, CARD_STROKE))
            .rounding(8.0)
            .min_size(Vec2::new(104.0, 34.0)),
    }
}

fn small_button<'a>(text: &'a str) -> egui::Button<'a> {
    egui::Button::new(egui::RichText::new(text).color(TEXT_DIM).size(12.5))
        .fill(Color32::from_rgb(32, 36, 46))
        .stroke(egui::Stroke::new(1.0_f32, CARD_STROKE))
        .rounding(7.0)
        .min_size(Vec2::new(0.0, 30.0))
}

fn chip(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::none()
        .fill(color.linear_multiply(0.16))
        .stroke(egui::Stroke::new(1.0_f32, color.linear_multiply(0.5)))
        .rounding(999.0)
        .inner_margin(egui::Margin::symmetric(10.0, 3.0))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).color(color).size(12.0).strong());
        });
}

fn log_lines(ui: &mut egui::Ui, log: &[String]) {
    egui::Frame::none()
        .fill(FIELD)
        .rounding(8.0)
        .stroke(egui::Stroke::new(1.0_f32, CARD_STROKE))
        .inner_margin(egui::Margin::same(8.0))
        .show(ui, |ui| {
            egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                for line in log {
                    ui.label(egui::RichText::new(line).color(TEXT_DIM).size(11.5).monospace());
                }
            });
        });
}

fn parse_rule_indices(s: &str) -> Vec<usize> {
    s.split([',', ' ', ';'])
        .filter_map(|t| t.trim().parse::<usize>().ok())
        .collect()
}

fn rule_indices_to_string(v: &[usize]) -> String {
    v.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")
}

// ---------------------------------------------------------------------------
// 向导页面
// ---------------------------------------------------------------------------

impl BuilderApp {
    fn page_welcome(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let h = ui.available_height().max(420.0);
        // 左侧品牌栏（NSIS/Inno 欢迎页侧边图）
        ui.horizontal(|ui| {
            let side_w = 260.0;
            let (side_rect, _) = ui.allocate_exact_size(Vec2::new(side_w, h - 8.0), Sense::hover());
            let p = ui.painter();
            let accent = self.accent();
            let n = 24;
            for i in 0..n {
                let t = (i as f32 + 0.5) / n as f32;
                let top = Color32::from_rgb(14, 20, 38);
                let bottom = Color32::from_rgb(24, 48, 106);
                let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
                let c = Color32::from_rgb(f(top.r(), bottom.r()), f(top.g(), bottom.g()), f(top.b(), bottom.b()));
                let y0 = side_rect.top() + side_rect.height() * (i as f32 / n as f32);
                let y1 = side_rect.top() + side_rect.height() * ((i + 1) as f32 / n as f32);
                p.rect_filled(Rect::from_min_max(Pos2::new(side_rect.left(), y0), Pos2::new(side_rect.right(), y1)), 0.0, c);
            }
            for i in 0..5 {
                let r = 160.0 - i as f32 * 28.0;
                p.circle_filled(
                    Pos2::new(side_rect.center().x, side_rect.top() + 90.0),
                    r,
                    accent.linear_multiply(0.10 - i as f32 * 0.018),
                );
            }
            p.text(
                Pos2::new(side_rect.center().x, side_rect.center().y - 26.0),
                egui::Align2::CENTER_CENTER,
                "MIEdit",
                FontId::proportional(54.0),
                Color32::WHITE,
            );
            p.text(
                Pos2::new(side_rect.center().x, side_rect.center().y + 22.0),
                egui::Align2::CENTER_CENTER,
                "Installer System",
                FontId::proportional(14.0),
                accent,
            );
            p.text(
                Pos2::new(side_rect.center().x, side_rect.bottom() - 26.0),
                egui::Align2::CENTER_CENTER,
                format!("v{}", miedit_core::VERSION),
                FontId::proportional(12.0),
                Color32::from_rgba_unmultiplied(210, 220, 240, 160),
            );

            // 右侧文案（侧边栏右方）——显式切回纵向布局
            let text_rect = Rect::from_min_max(
                Pos2::new(side_rect.right() + 20.0, side_rect.top() + 8.0),
                Pos2::new(ui.max_rect().right(), side_rect.bottom() - 8.0),
            );
            ui.allocate_ui_at_rect(text_rect, |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.add_space(14.0);
                    ui.label(egui::RichText::new("欢迎使用 MIEdit 安装包构建向导").color(TEXT).size(24.0).strong());
                    ui.add_space(14.0);
                    ui.label(
                        egui::RichText::new(
                            "本向导将指导你创建一个新 MIEdit 自解压安装程序的完整配置。结果将生成一个单文件安装程序 EXE，包含现代化安装向导与自动生成的卸载程序。",
                        )
                        .color(TEXT_DIM)
                        .size(14.0),
                    );
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new(
                            "向导并不会涵盖 MIEdit 的所有功能。生成的 MI 脚本（.mi，自研轻量脚本语言，TOML 语法）可在「MI 脚本」页像 VSCode 一样直接编辑，或使用 miedit-compiler 命令行编译。",
                        )
                        .color(TEXT_DIM)
                        .size(14.0),
                    );
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new("单击「下一步」继续，或单击「取消」退出此向导。")
                            .color(TEXT)
                            .size(14.0),
                    );
                    ui.add_space(24.0);
                    let mut open = false;
                    ui.checkbox(
                        &mut open,
                        egui::RichText::new("打开现有项目（.mi / .toml）").color(TEXT).size(13.5),
                    );
                    if open {
                        if let Some(p) = rfd::FileDialog::new()
                            .add_filter("MI 脚本（.mi）", &["mi", "toml"])
                            .pick_file()
                        {
                            if let Err(e) = self.open_project(&p, ctx) {
                                self.set_flash(format!("打开项目失败: {e:#}"));
                            } else {
                                self.step = Step::AppInfo;
                            }
                        }
                    }
                });
            });
        });
    }

    fn page_app_info(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let base = ui.available_width() - 40.0;
        field_row(ui, "应用程序名称 *：", &mut self.cfg.app.name, "例如：我的应用", base, 0.0);
        ui.horizontal(|ui| {
            ui.add_sized([150.0, 0.0], egui::Label::new(egui::RichText::new("应用程序版本：").color(TEXT).size(13.5)));
            text_input(ui, &mut self.cfg.app.version, "1.0.0", 180.0);
            ui.add_sized([90.0, 0.0], egui::Label::new(egui::RichText::new("发布者：").color(TEXT).size(13.5)));
            text_input(ui, &mut self.cfg.app.publisher, "发布者名称", base - 340.0);
        });
        field_row(ui, "应用程序网站：", &mut self.cfg.app.url, "https://example.com（可选）", base, 0.0);
        field_label(ui, "主程序（包内路径，用于快捷方式 / 图标 / 安装后运行）");
        ui.horizontal(|ui| {
            text_input(ui, &mut self.cfg.app.exe, "例如 MyApp.exe，可为空", base - 90.0);
            if ui.add(small_button("浏览...")).clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("可执行文件", &["exe"]).pick_file() {
                    self.cfg.app.exe = p.display().to_string();
                }
            }
        });
        ui.add_space(6.0);
        ui.checkbox(
            &mut self.cfg.app.per_machine,
            egui::RichText::new("每机器安装（写入 HKLM 与公共开始菜单，启动时请求管理员权限）")
                .color(TEXT)
                .size(13.5),
        );
        let _ = ctx;
    }

    fn page_options(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.columns(2, |cols| {
            // 左：外观选项
            let col = &mut cols[0];
            let base = col.available_width() - 8.0;
            field_label(col, "安装程序图标（.ico，替换成品 EXE 图标 + 窗口图标）");
            col.horizontal(|ui| {
                let mut icon_buf = self.cfg.app.icon.clone().unwrap_or_default();
                text_input(ui, &mut icon_buf, "未设置", base - 90.0);
                self.cfg.app.icon = if icon_buf.trim().is_empty() { None } else { Some(icon_buf) };
                if ui.add(small_button("浏览...")).clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("图标", &["ico"]).pick_file() {
                        self.cfg.app.icon = Some(p.display().to_string());
                    }
                }
            });
            col.add_space(8.0);
            field_label(col, "全窗口背景图片");
            col.horizontal(|ui| {
                let mut bg_buf = self.cfg.ui.background.clone().unwrap_or_default();
                text_input(ui, &mut bg_buf, "未设置（深色渐变）", base - 190.0);
                self.cfg.ui.background = if bg_buf.trim().is_empty() { None } else { Some(bg_buf) };
                if ui.add(small_button("选择图片")).clicked() {
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter("图片", &["png", "jpg", "jpeg", "bmp", "webp", "gif", "tga", "tiff", "ico"])
                        .pick_file()
                    {
                        self.cfg.ui.background = Some(p.display().to_string());
                        self.load_background(ctx);
                    }
                }
                if self.cfg.ui.background.is_some() && ui.add(small_button("清除")).clicked() {
                    self.cfg.ui.background = None;
                    self.bg_tex = None;
                }
            });
            col.horizontal(|ui| {
                ui.add_sized([80.0, 0.0], egui::Label::new(egui::RichText::new("显示模式：").color(TEXT).size(13.5)));
                for (style, label) in [("cover", "等比填满"), ("contain", "等比完整"), ("stretch", "拉伸")] {
                    let selected = self.cfg.ui.background_style == style;
                    if ui
                        .add(egui::SelectableLabel::new(
                            selected,
                            egui::RichText::new(label)
                                .color(if selected { Color32::WHITE } else { TEXT_DIM })
                                .size(13.0),
                        ))
                        .clicked()
                    {
                        self.cfg.ui.background_style = style.to_string();
                    }
                }
            });
            col.horizontal(|ui| {
                ui.add_sized([80.0, 0.0], egui::Label::new(egui::RichText::new("主题色：").color(TEXT).size(13.5)));
                for (hex, tip) in ACCENT_PRESETS {
                    let on = self.cfg.ui.accent.eq_ignore_ascii_case(hex);
                    let [r, g, b] = miedit_core::util::parse_hex_color(hex).unwrap_or([255, 255, 255]);
                    let color = Color32::from_rgb(r, g, b);
                    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
                    ui.painter().rect_filled(rect, 6.0, color);
                    if on {
                        ui.painter().rect_stroke(rect, 6.0, egui::Stroke::new(2.0_f32, Color32::WHITE));
                    }
                    if resp.on_hover_text(*tip).clicked() {
                        self.cfg.ui.accent = hex.to_string();
                    }
                }
                text_input(ui, &mut self.cfg.ui.accent, "#RRGGBB", 100.0);
            });
            col.horizontal(|ui| {
                ui.add_sized([80.0, 0.0], egui::Label::new(egui::RichText::new("窗口尺寸：").color(TEXT).size(13.5)));
                ui.add_sized([100.0, 0.0], egui::DragValue::new(&mut self.cfg.ui.window_size[0]).range(720.0..=1600.0).speed(10.0).prefix("宽 "));
                ui.add_sized([100.0, 0.0], egui::DragValue::new(&mut self.cfg.ui.window_size[1]).range(520.0..=1200.0).speed(10.0).prefix("高 "));
            });
            col.horizontal(|ui| {
                ui.add_sized([80.0, 0.0], egui::Label::new(egui::RichText::new("压缩方式：").color(TEXT).size(13.5)));
                let mut comp = self.cfg.install.compression.clone();
                let comp_label = match comp.as_str() {
                    "solid" => "Deflate Solid（整包，体积更小）",
                    "lzma" => "LZMA（条目级）",
                    "lzma-solid" => "LZMA Solid（整包，NSIS 默认风格，体积最小）",
                    "brotli" => "Brotli（条目级）",
                    "brotli-solid" => "Brotli Solid（整包）",
                    _ => "Deflate（条目级，速度快）",
                };
                egui::ComboBox::from_id_salt("compression")
                    .width(300.0)
                    .selected_text(comp_label)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut comp, "deflate".to_string(), "Deflate（条目级，速度快）");
                        ui.selectable_value(&mut comp, "solid".to_string(), "Deflate Solid（整包，体积更小）");
                        ui.selectable_value(&mut comp, "lzma".to_string(), "LZMA（条目级，NSIS 旗舰算法）");
                        ui.selectable_value(&mut comp, "lzma-solid".to_string(), "LZMA Solid（整包，NSIS 默认风格，体积最小）");
                        ui.selectable_value(&mut comp, "brotli".to_string(), "Brotli（条目级，现代算法）");
                        ui.selectable_value(&mut comp, "brotli-solid".to_string(), "Brotli Solid（整包，现代算法）");
                    });
                self.cfg.install.compression = comp;
            });
            col.add_space(8.0);
            field_label(col, "输出文件名（第 9 步可更改完整路径）");
            let hint = self.output_path().display().to_string();
            text_input(col, &mut self.output, hint.as_str(), base);

            // 右：实时预览
            let rcol = &mut cols[1];
            rcol.add_space(2.0);
            rcol.label(egui::RichText::new("实时预览").color(TEXT).size(15.0).strong());
            rcol.add_space(4.0);
            let pw = rcol.available_width() - 8.0;
            preview::draw(rcol, self.bg_tex.as_ref(), &self.cfg, pw);
        });
    }

    fn page_dir_license(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let base = ui.available_width() - 40.0;
        field_label(ui, "应用程序默认目录模板");
        ui.horizontal(|ui| {
            let mut dir_tpl = self.cfg.install.dir_template.clone();
            let combo_w = base - 90.0;
            egui::ComboBox::from_id_salt("dir-tpl")
                .width(combo_w - 10.0)
                .selected_text(&dir_tpl)
                .show_ui(ui, |ui| {
                    for tpl in [
                        "{autopf}\\{app_name}",
                        "{localappdata}\\Programs\\{app_name}",
                        "{appdata}\\{app_name}",
                    ] {
                        ui.selectable_value(&mut dir_tpl, tpl.to_string(), tpl);
                    }
                });
            self.cfg.install.dir_template = dir_tpl;
        });
        ui.label(egui::RichText::new(WINDOW_HINT).color(Color32::from_rgb(110, 116, 130)).size(11.5));
        ui.add_space(6.0);
        ui.checkbox(
            &mut self.cfg.ui.show_dir_page,
            egui::RichText::new("允许用户更改应用程序目录（显示目录选择页）").color(TEXT).size(13.5),
        );
        ui.add_space(10.0);
        field_label(ui, "授权（许可）文件");
        ui.horizontal(|ui| {
            let mut lic_buf = self.cfg.ui.license.clone().unwrap_or_default();
            text_input(ui, &mut lic_buf, "未设置（不显示许可页面）", base - 90.0);
            self.cfg.ui.license = if lic_buf.trim().is_empty() { None } else { Some(lic_buf) };
            if ui.add(small_button("浏览...")).clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("文本", &["txt", "md", "rtf"]).pick_file() {
                    self.cfg.ui.license = Some(p.display().to_string());
                    self.load_license_preview();
                }
            }
            if self.cfg.ui.license.is_some() && ui.add(small_button("清除")).clicked() {
                self.cfg.ui.license = None;
                self.license_preview = None;
            }
        });
        if !self.cfg.ui.license.as_deref().unwrap_or("").is_empty() {
            field_label(ui, "选择用户如何接受许可协议：");
            ui.horizontal(|ui| {
                let mut style = if self.cfg.ui.license_scroll_required { 1 } else { 0 };
                if ui
                    .add(egui::RadioButton::new(style == 0, egui::RichText::new("勾选框：勾选“我接受”后继续").color(TEXT).size(13.5)))
                    .clicked()
                {
                    style = 0;
                }
                if ui
                    .add(egui::RadioButton::new(style == 1, egui::RichText::new("强制滚动：滚到底部才可接受（Inno 风格）").color(TEXT).size(13.5)))
                    .clicked()
                {
                    style = 1;
                }
                self.cfg.ui.license_scroll_required = style == 1;
                let _ = style;
            });
            if let Some(text) = &self.license_preview {
                egui::Frame::none()
                    .fill(FIELD)
                    .rounding(8.0)
                    .stroke(egui::Stroke::new(1.0_f32, CARD_STROKE))
                    .inner_margin(egui::Margin::same(10.0))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.label(egui::RichText::new(text).color(TEXT_DIM).size(12.0));
                        });
                    });
            }
        }
        let _ = ctx;
    }

    fn page_files_components(&mut self, ui: &mut egui::Ui) {
        let accent = self.accent();
        let mut show_components = self.cfg.ui.show_components_page.unwrap_or(true);
        ui.checkbox(
            &mut show_components,
            egui::RichText::new("允许用户选择要安装的组件（组件页）").color(TEXT).size(13.5),
        );
        self.cfg.ui.show_components_page = Some(show_components);
        ui.add_space(6.0);

        ui.columns(2, |cols| {
            // 左：组件
            let col = &mut cols[0];
            field_label(col, "安装组件（NSIS Section / Inno Components）：");
            let inner_w = col.available_width() - 12.0;
            egui::ScrollArea::vertical()
                .auto_shrink([false, true])
                .max_height(300.0)
                .show(col, |ui| {
                    let mut comp_remove: Option<usize> = None;
                    for (i, comp) in self.cfg.components.iter_mut().enumerate() {
                        egui::Frame::none()
                            .fill(CARD)
                            .rounding(8.0)
                            .stroke(egui::Stroke::new(1.0_f32, CARD_STROKE))
                            .inner_margin(egui::Margin::symmetric(8.0, 6.0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    text_input(ui, &mut comp.name, "组件名称", inner_w - 170.0);
                                    ui.checkbox(&mut comp.required, egui::RichText::new("必装").color(TEXT_DIM).size(12.0));
                                    if ui.small_button(egui::RichText::new("×").color(RED)).clicked() {
                                        comp_remove = Some(i);
                                    }
                                });
                                text_input(ui, &mut comp.description, "描述", inner_w);
                                text_input(ui, &mut comp.group, "分组（可选，如 Tools/Compression）", inner_w);
                                let mut files_text = rule_indices_to_string(&comp.files);
                                ui.horizontal(|ui| {
                                    text_input(ui, &mut files_text, "0,2", 90.0);
                                    comp.files = parse_rule_indices(&files_text);
                                    ui.label(egui::RichText::new("← 文件规则编号").color(Color32::from_rgb(110, 116, 130)).size(11.0));
                                });
                            });
                    }
                    if let Some(i) = comp_remove {
                        self.cfg.components.remove(i);
                    }
                });
            col.horizontal(|ui| {
                if ui.add(small_button("＋ 添加组件")).clicked() {
                    self.cfg.components.push(miedit_core::config::ComponentSpec::default());
                }
            });

            // 左下：安装类型（NSIS InstallTypes，路线图步骤 10）
            col.add_space(10.0);
            field_label(col, "安装类型（NSIS InstallTypes，组件页顶部单选）：");
            let mut type_remove: Option<usize> = None;
            let mut type_set_default: Option<usize> = None;
            egui::ScrollArea::vertical()
                .auto_shrink([false, true])
                .max_height(130.0)
                .show(col, |ui| {
                    for (i, t) in self.cfg.install_types.iter_mut().enumerate() {
                        egui::Frame::none()
                            .fill(CARD)
                            .rounding(8.0)
                            .stroke(egui::Stroke::new(1.0_f32, CARD_STROKE))
                            .inner_margin(egui::Margin::symmetric(8.0, 6.0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    text_input(ui, &mut t.name, "Typical", inner_w - 170.0);
                                    if ui
                                        .checkbox(
                                            &mut t.default,
                                            egui::RichText::new("默认").color(TEXT_DIM).size(12.0),
                                        )
                                        .changed()
                                        && t.default
                                    {
                                        type_set_default = Some(i);
                                    }
                                    if ui.small_button(egui::RichText::new("×").color(RED)).clicked() {
                                        type_remove = Some(i);
                                    }
                                });
                                let mut comps_text = rule_indices_to_string(&t.components);
                                ui.horizontal(|ui| {
                                    text_input(ui, &mut comps_text, "0,2", 90.0);
                                    t.components = parse_rule_indices(&comps_text);
                                    ui.label(egui::RichText::new("← 默认勾选的组件编号").color(Color32::from_rgb(110, 116, 130)).size(11.0));
                                });
                            });
                    }
                    if let Some(i) = type_remove {
                        self.cfg.install_types.remove(i);
                    }
                });
            if let Some(k) = type_set_default {
                // default 互斥：标记一个后其余自动取消
                for (j, t) in self.cfg.install_types.iter_mut().enumerate() {
                    t.default = j == k;
                }
            }
            col.horizontal(|ui| {
                if ui.add(small_button("＋ 添加安装类型")).clicked() {
                    self.cfg.install_types.push(miedit_core::config::InstallTypeSpec::default());
                }
            });

            // 右：文件规则
            let rcol = &mut cols[1];
            field_label(rcol, "应用程序文件（source → 包内目录）：");
            let rinner_w = rcol.available_width() - 12.0;
            egui::ScrollArea::vertical()
                .auto_shrink([false, true])
                .max_height(300.0)
                .show(rcol, |ui| {
                    let mut changed = false;
                    let mut remove_idx: Option<usize> = None;
                    for (i, spec) in self.cfg.files.iter_mut().enumerate() {
                        egui::Frame::none()
                            .fill(CARD)
                            .rounding(8.0)
                            .stroke(egui::Stroke::new(1.0_f32, CARD_STROKE))
                            .inner_margin(egui::Margin::symmetric(8.0, 6.0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.monospace(egui::RichText::new(format!("[{}]", i)).color(accent).size(12.0));
                                    let before = spec.source.clone();
                                    text_input(ui, &mut spec.source, "源（文件/目录/通配符 **）", rinner_w - 200.0);
                                    if spec.source != before {
                                        changed = true;
                                    }
                                    text_input(ui, &mut spec.dest, "包内目录", 110.0);
                                    if ui.small_button(egui::RichText::new("×").color(RED)).clicked() {
                                        remove_idx = Some(i);
                                        changed = true;
                                    }
                                });
                            });
                    }
                    if let Some(i) = remove_idx {
                        self.cfg.files.remove(i);
                    }
                    if changed {
                        self.refresh_files_stat();
                    }
                });
            rcol.horizontal(|ui| {
                if ui.add(small_button("＋ 添加文件")).clicked() {
                    if let Some(paths) = rfd::FileDialog::new().pick_files() {
                        for p in paths {
                            self.cfg.files.push(miedit_core::config::FileSpec {
                                source: p.display().to_string(),
                                dest: String::new(),
                            });
                        }
                        self.refresh_files_stat();
                    }
                }
                if ui.add(small_button("＋ 添加目录")).clicked() {
                    if let Some(p) = rfd::FileDialog::new().pick_folder() {
                        self.cfg.files.push(miedit_core::config::FileSpec {
                            source: p.display().to_string(),
                            dest: String::new(),
                        });
                        self.refresh_files_stat();
                    }
                }
                if ui.add(small_button("＋ 添加通配符")).clicked() {
                    self.cfg.files.push(miedit_core::config::FileSpec::default());
                }
            });
            let (n, bytes) = self.files_stat;
            rcol.label(
                egui::RichText::new(format!("共 {n} 个文件 · 未压缩 {}", miedit_core::util::human_size(bytes)))
                    .color(TEXT_DIM)
                    .size(11.5),
            );
        });
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new("提示：未被任何组件引用的文件规则始终安装；组件的“文件规则编号”引用上表 [n]。")
                .color(Color32::from_rgb(110, 116, 130))
                .size(11.5),
        );
    }

    fn page_shortcuts(&mut self, ui: &mut egui::Ui) {
        let base = ui.available_width() - 40.0;
        ui.checkbox(
            &mut self.cfg.shortcuts.start_menu,
            egui::RichText::new("在「开始菜单」程序文件夹中创建快捷方式（自动含卸载快捷方式）").color(TEXT).size(13.5),
        );
        let mut name = self.cfg.shortcuts.name.clone().unwrap_or_default();
        field_row(ui, "开始菜单文件夹名称：", &mut name, "默认应用名称", base, 0.0);
        self.cfg.shortcuts.name = if name.is_empty() { None } else { Some(name) };
        ui.add_space(6.0);
        ui.checkbox(&mut self.cfg.shortcuts.desktop, egui::RichText::new("允许用户创建桌面快捷方式（桌面快捷方式）").color(TEXT).size(13.5));
        field_row(ui, "主程序启动参数：", &mut self.cfg.shortcuts.run_args, "可空", base, 0.0);
    }

    fn page_run_after(&mut self, ui: &mut egui::Ui) {
        let base = ui.available_width() - 40.0;
        ui.label(
            egui::RichText::new("任务将在完成页显示为复选框，由用户决定是否运行。为空时：若主程序存在则显示“运行主程序”。")
                .color(TEXT_DIM)
                .size(12.5),
        );
        ui.add_space(6.0);
        let accent = self.accent();
        let mut task_remove: Option<usize> = None;
        for (i, task) in self.cfg.install.run_tasks.iter_mut().enumerate() {
            egui::Frame::none()
                .fill(CARD)
                .rounding(8.0)
                .stroke(egui::Stroke::new(1.0_f32, CARD_STROKE))
                .inner_margin(egui::Margin::symmetric(8.0, 6.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.monospace(egui::RichText::new(format!("#{}", i + 1)).color(accent).size(12.0));
                        text_input(ui, &mut task.name, "复选框文案（支持 {app} {version}）", base - 230.0);
                        ui.checkbox(&mut task.default_checked, egui::RichText::new("默认勾选").color(TEXT_DIM).size(12.5));
                        if ui.small_button(egui::RichText::new("×").color(RED)).clicked() {
                            task_remove = Some(i);
                        }
                    });
                    ui.horizontal(|ui| {
                        text_input(ui, &mut task.exe, "程序（包内路径，exe/md/txt 均可）", base - 170.0);
                        text_input(ui, &mut task.args, "参数", 160.0);
                    });
                });
        }
        if let Some(i) = task_remove {
            self.cfg.install.run_tasks.remove(i);
        }
        if ui.add(small_button("＋ 添加任务")).clicked() {
            self.cfg.install.run_tasks.push(miedit_core::config::RunTask::default());
        }
    }

    fn page_uninstall_reg(&mut self, ui: &mut egui::Ui) {
        let base = ui.available_width() - 40.0;
        field_label(ui, "用户数据 / 配置目录模板（安装时记录；卸载确认页可勾选删除）：");
        let mut remove_ud: Option<usize> = None;
        for (i, tpl) in self.cfg.install.user_data_dirs.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                text_input(ui, tpl, "{appdata}\\{app_name}", base - 46.0);
                if ui.small_button(egui::RichText::new("×").color(RED)).clicked() {
                    remove_ud = Some(i);
                }
            });
        }
        if let Some(i) = remove_ud {
            self.cfg.install.user_data_dirs.remove(i);
        }
        if ui.add(small_button("＋ 添加目录")).clicked() {
            self.cfg.install.user_data_dirs.push(String::new());
        }
        ui.add_space(10.0);
        field_label(ui, "注册表键（卸载时一并清理）：");
        let mut reg_remove: Option<usize> = None;
        for (i, row) in self.reg_rows.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                text_input(ui, &mut row.key, "HKCU\\Software\\厂商\\应用", 200.0);
                text_input(ui, &mut row.values, "值，每行 名称=数据（{install_dir} 可用）", base - 280.0);
                if ui.small_button(egui::RichText::new("×").color(RED)).clicked() {
                    reg_remove = Some(i);
                }
            });
        }
        if let Some(i) = reg_remove {
            self.reg_rows.remove(i);
        }
        if ui.add(small_button("＋ 添加注册表键")).clicked() {
            self.reg_rows.push(RegRow { key: String::new(), values: String::new() });
        }
        ui.add_space(10.0);
        // 环境变量（路线图步骤 27：NSIS WriteEnvStr 惯例 / Inno ChangesEnvironment 对齐）
        let accent = self.accent();
        field_label(ui, "环境变量（安装时写入并广播 WM_SETTINGCHANGE；卸载时自动还原/清理）：");
        let mut env_remove: Option<usize> = None;
        for (i, e) in self.cfg.install.env.iter_mut().enumerate() {
            egui::Frame::none()
                .fill(CARD)
                .rounding(8.0)
                .stroke(egui::Stroke::new(1.0_f32, CARD_STROKE))
                .inner_margin(egui::Margin::symmetric(8.0, 6.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.monospace(egui::RichText::new(format!("#{}", i + 1)).color(accent).size(12.0));
                        text_input(ui, &mut e.name, "变量名（如 PATH / JAVA_HOME）", 180.0);
                        text_input(ui, &mut e.value, "值（{install_dir} 可用；PATH 模式为单个路径段）", base - 330.0);
                        let part = e.part.clone();
                        let part_label = match part.as_str() {
                            "path_append" => "追加",
                            "path_prepend" => "前置",
                            _ => "直接设值",
                        };
                        egui::ComboBox::from_id_salt(format!("env-part-{i}"))
                            .width(96.0)
                            .selected_text(part_label)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut e.part, "all".to_string(), "直接设值");
                                ui.selectable_value(&mut e.part, "path_append".to_string(), "追加到 PATH");
                                ui.selectable_value(&mut e.part, "path_prepend".to_string(), "前置到 PATH");
                            });
                        if part != e.part {
                            // 切换 PATH 模式时默认补一个语义化默认值提示
                            if e.part != "all" && e.value.is_empty() {
                                e.value = "{install_dir}".into();
                            }
                        }
                        let scope = e.scope.clone();
                        let scope_label = if scope == "machine" { "机器级" } else { "用户级" };
                        egui::ComboBox::from_id_salt(format!("env-scope-{i}"))
                            .width(72.0)
                            .selected_text(scope_label)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut e.scope, "user".to_string(), "用户级");
                                ui.selectable_value(&mut e.scope, "machine".to_string(), "机器级");
                            });
                        if ui.small_button(egui::RichText::new("×").color(RED)).clicked() {
                            env_remove = Some(i);
                        }
                    });
                });
        }
        if let Some(i) = env_remove {
            self.cfg.install.env.remove(i);
        }
        if ui.add(small_button("＋ 添加环境变量")).clicked() {
            self.cfg.install.env.push(miedit_core::config::EnvSpec::default());
        }
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("提示：PATH 追加/前置按 \";\" 列表去重合并（不覆盖已有内容），卸载时只移除本安装写入的段；\"直接设值\" 卸载时还原安装前的原值。")
                .color(TEXT_DIM)
                .size(12.0),
        );
    }

    /// VSCode 式脚本编辑页（路线图步骤 32）：菜单栏 + 标签栏 + 面包屑 +
    /// Dark+ 代码区（行号/当前行/查找命中/错误行高亮）+ minimap + 查找替换 +
    /// 命令面板 + 转到行 + 状态栏 + 问题面板 + 自动缩进/括号补全。
    fn page_script(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let editor_id = egui::Id::new("mi-script-editor");
        let tab_label = self.tab_label();
        // 实时校验（一处解析：状态栏 / 问题面板 / 错误行高亮共用）
        let parse_result = toml::from_str::<Config>(&self.script_text);
        let error_pos =
            parse_result.as_ref().err().and_then(|e| scriptide::parse_toml_error_pos(&e.to_string()));
        let has_error = parse_result.is_err();
        // MI 脚本 v2（步骤 33）：解析成功后对组件指令做静态校验
        // （必填字段/dword 合法性/hive 前缀），问题面板与状态栏共用
        let inst_errs: Vec<String> = match &parse_result {
            Ok(cfg) => cfg
                .components
                .iter()
                .enumerate()
                .flat_map(|(ci, c)| {
                    miedit_core::micmd::validate(&c.code).into_iter().map(move |e| {
                        let cname = if c.name.trim().is_empty() {
                            format!("#{}", ci + 1)
                        } else {
                            c.name.clone()
                        };
                        format!("组件「{}」{e}", cname)
                    })
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        let accent = self.accent();

        // 上一帧光标（行号高亮 / 面包屑用；编辑后的新光标由状态栏重读）
        let prev_caret = egui::TextEdit::load_state(ctx, editor_id)
            .and_then(|st| st.cursor.char_range())
            .map(|r| r.primary.index);
        let caret_line_prev =
            prev_caret.map(|c| scriptide::line_of_char(&self.script_text, c)).unwrap_or(1);

        let mut actions: Vec<IdeAction> = Vec::new();

        // ---- VSCode 快捷键（仅脚本页生效）----
        ctx.input(|i| {
            let m = i.modifiers;
            let pressed = |k: egui::Key| i.key_pressed(k);
            let a = if m.ctrl && m.shift && pressed(egui::Key::P) {
                Some(IdeAction::TogglePalette)
            } else if m.ctrl && pressed(egui::Key::F) {
                Some(IdeAction::ToggleFind)
            } else if m.ctrl && pressed(egui::Key::H) {
                Some(IdeAction::ToggleReplace)
            } else if m.ctrl && pressed(egui::Key::G) {
                Some(IdeAction::GotoLine)
            } else if m.ctrl && pressed(egui::Key::S) {
                Some(IdeAction::Save)
            } else if m.alt && pressed(egui::Key::Z) {
                Some(IdeAction::ToggleWrap)
            } else {
                None
            };
            if let Some(a) = a {
                actions.push(a);
            }
        });

        // Esc 分层关闭：命令面板 → 转到行 → 查找（VSCode 语义）
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.ide.palette.open {
                self.ide.palette.open = false;
            } else if self.ide.goto.open {
                self.ide.goto.open = false;
            } else if self.ide.find.open {
                self.ide.find.open = false;
            }
        }

        // ---- 菜单栏（VSCode 顶部菜单；RefCell 规避嵌套闭包可变借用）----
        let picked = std::cell::RefCell::new(None::<IdeAction>);
        egui::menu::bar(ui, |ui| {
            ui.menu_button("文件", |ui| {
                for (label, a) in [
                    ("保存脚本　Ctrl+S", IdeAction::Save),
                    ("另存为脚本...", IdeAction::SaveAs),
                    ("打开项目...", IdeAction::OpenProject),
                ] {
                    if ui.button(label).clicked() {
                        *picked.borrow_mut() = Some(a);
                        ui.close_menu();
                    }
                }
            });
            ui.menu_button("编辑", |ui| {
                for (label, a) in [
                    ("查找　Ctrl+F", IdeAction::ToggleFind),
                    ("替换　Ctrl+H", IdeAction::ToggleReplace),
                    ("转到行　Ctrl+G", IdeAction::GotoLine),
                ] {
                    if ui.button(label).clicked() {
                        *picked.borrow_mut() = Some(a);
                        ui.close_menu();
                    }
                }
            });
            ui.menu_button("查看", |ui| {
                let wrap_label = if self.ide.wrap {
                    "自动换行（开）　Alt+Z"
                } else {
                    "自动换行（关）　Alt+Z"
                };
                for (label, a) in [
                    ("命令面板　Ctrl+Shift+P", IdeAction::TogglePalette),
                    (wrap_label, IdeAction::ToggleWrap),
                ] {
                    if ui.button(label).clicked() {
                        *picked.borrow_mut() = Some(a);
                        ui.close_menu();
                    }
                }
            });
            ui.menu_button("工具", |ui| {
                for (label, a) in [
                    ("应用修改到向导", IdeAction::Apply),
                    ("重新生成脚本（丢弃编辑）", IdeAction::Regenerate),
                    ("转到构建页", IdeAction::OpenBuild),
                ] {
                    if ui.button(label).clicked() {
                        *picked.borrow_mut() = Some(a);
                        ui.close_menu();
                    }
                }
            });
            ui.menu_button("帮助", |ui| {
                if ui.button("MI 脚本语法说明").clicked() {
                    *picked.borrow_mut() = Some(IdeAction::HelpSyntax);
                    ui.close_menu();
                }
            });
        });
        if let Some(a) = picked.into_inner() {
            actions.push(a);
        }

        // ---- 标签栏（VSCode 编辑器标签：文件图标 + 名称 + 未保存圆点）----
        egui::Frame::none()
            .fill(FIELD)
            .rounding(8.0)
            .stroke(egui::Stroke::new(1.0_f32, CARD_STROKE))
            .inner_margin(egui::Margin::symmetric(10.0, 5.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    egui::Frame::none()
                        .fill(scriptide::VSC_BG)
                        .rounding(6.0)
                        .inner_margin(egui::Margin::symmetric(10.0, 6.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let (r, _) =
                                    ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                                let p = ui.painter_at(r);
                                p.rect_filled(r, 3.0, scriptide::VSC_SECTION);
                                p.text(
                                    r.center(),
                                    egui::Align2::CENTER_CENTER,
                                    "T",
                                    FontId::proportional(10.0),
                                    Color32::WHITE,
                                );
                                ui.label(
                                    egui::RichText::new(&tab_label).color(TEXT).size(13.0).strong(),
                                );
                                if self.script_dirty {
                                    // VSCode 未保存圆点
                                    let (r, _) =
                                        ui.allocate_exact_size(Vec2::new(9.0, 9.0), Sense::hover());
                                    ui.painter_at(r).circle_filled(r.center(), 3.5, AMBER);
                                }
                            });
                        });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // 面包屑：文件 › [当前节]（VSCode Breadcrumbs）
                        if let Some((sec, _)) =
                            scriptide::breadcrumb_section(&self.script_text, caret_line_prev)
                        {
                            ui.label(
                                egui::RichText::new(format!("› [{sec}]"))
                                    .color(Color32::from_rgb(140, 146, 160))
                                    .size(11.5),
                            );
                        }
                        ui.label(
                            egui::RichText::new(&tab_label)
                                .color(Color32::from_rgb(120, 126, 140))
                                .size(11.5),
                        );
                    });
                });
            });

        // ---- 工具栏（快捷操作行）----
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if self.script_dirty {
                chip(ui, "有未应用的修改", AMBER);
            }
            if ui.add(small_button("应用修改到向导")).clicked() {
                actions.push(IdeAction::Apply);
            }
            if ui.add(small_button("保存脚本")).clicked() {
                actions.push(IdeAction::Save);
            }
            if ui.add(small_button("另存为脚本...")).clicked() {
                actions.push(IdeAction::SaveAs);
            }
            if ui.add(small_button("重新生成（丢弃编辑）")).clicked() {
                actions.push(IdeAction::Regenerate);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(
                        "Ctrl+F 查找 · Ctrl+H 替换 · Ctrl+G 转到行 · Ctrl+Shift+P 命令面板",
                    )
                    .color(Color32::from_rgb(110, 116, 130))
                    .size(11.0),
                );
            });
        });
        ui.add_space(6.0);

        // ---- 编辑器主体（VSCode Dark+ 代码区 + 行号 + 高亮层 + minimap）----
        let query = self.ide.find.query.clone();
        let matches = if query.is_empty() {
            Vec::new()
        } else {
            scriptide::find_matches(&self.script_text, &query, self.ide.find.case_sensitive)
        };
        let n_matches = matches.len();
        // 查询变化时回到第一个命中（VSCode 行为：输入即跳转）
        if self.ide.find.query != self.ide.last_query {
            self.ide.last_query = self.ide.find.query.clone();
            if n_matches > 0 {
                self.ide.find.current = 0;
            }
        }
        let hl = scriptide::Highlight {
            current: if matches.is_empty() {
                None
            } else {
                Some(self.ide.find.current.min(matches.len() - 1))
            },
            matches,
            caret_line: Some(caret_line_prev),
            error_line: error_pos.map(|(l, _)| l),
        };
        egui::Frame::none()
            .fill(scriptide::VSC_BG)
            .rounding(8.0)
            .stroke(egui::Stroke::new(1.0_f32, CARD_STROKE))
            .inner_margin(egui::Margin::same(8.0))
            .show(ui, |ui| {
                // 编辑前采集编辑意图（供自动缩进 / 括号补全后处理）
                let events = ctx.input(|i| i.events.clone());
                let intents = scriptide::EditIntents::collect(&events);

                let mut layouter =
                    |ui: &egui::Ui, text: &str, wrap_width: f32| -> std::sync::Arc<egui::Galley> {
                        let mut job = scriptide::mi_script_job_hl(text, Some(caret_line_prev));
                        job.wrap.max_width = if self.ide.wrap { wrap_width } else { f32::INFINITY };
                        ui.fonts(|f| f.layout_job(job))
                    };
                let editor_w = (ui.available_width() - 104.0).max(320.0);
                let out = ui
                    .horizontal(|ui| {
                        let out = egui::TextEdit::multiline(&mut self.script_text)
                            .id_salt("mi-script-editor")
                            .font(egui::TextStyle::Monospace)
                            .code_editor()
                            .desired_rows(22)
                            .desired_width(editor_w)
                            .frame(false)
                            .layouter(&mut layouter)
                            .show(ui);
                        // 高亮层：当前行 / 查找命中 / 错误行
                        scriptide::paint_highlight(ui, &out, &self.script_text, &hl);
                        // minimap（点击跳行，VSCode 右侧缩略图）
                        let mm = scriptide::minimap(
                            ui,
                            &self.script_text,
                            caret_line_prev,
                            92.0,
                            out.response.rect.height(),
                            accent,
                        );
                        if let Some(line) = mm {
                            actions.push(IdeAction::JumpLine(line));
                        }
                        self.ide.editor_rect = out.text_clip_rect;
                        out
                    })
                    .inner;

                // 自动缩进 / Tab 转空格 / 括号引号补全与跨越（后处理，仅编辑器聚焦时）
                if out.response.has_focus() {
                    if let Some(caret) = scriptide::caret_char(&out) {
                        let edit = if intents.enter {
                            scriptide::auto_indent_after_newline(&self.script_text, caret, 4)
                        } else if intents.tab {
                            scriptide::expand_tab_at(&self.script_text, caret, 4)
                        } else if let Some(c) = intents.quote {
                            if self.script_text.chars().nth(caret) == Some(c) {
                                scriptide::skip_over_closer(&self.script_text, caret, c)
                            } else {
                                scriptide::auto_close_pair(&self.script_text, caret, c)
                            }
                        } else if let Some(c) = intents.open {
                            scriptide::auto_close_pair(&self.script_text, caret, c)
                        } else if let Some(c) = intents.closer {
                            scriptide::skip_over_closer(&self.script_text, caret, c)
                        } else {
                            None
                        };
                        if let Some((new_text, new_caret)) = edit {
                            self.script_text = new_text;
                            let mut st = out.state.clone();
                            st.cursor.set_char_range(Some(egui::text::CCursorRange::one(
                                egui::text::CCursor::new(new_caret),
                            )));
                            st.store(ctx, editor_id);
                        }
                    }
                }
            });

        // ---- 浮层：查找/替换、命令面板、转到行 ----
        let editor_rect = self.ide.editor_rect;
        if let Some(a) = scriptide::show_find(ctx, &mut self.ide.find, editor_rect, n_matches) {
            actions.push(a);
        }
        let cmds = Self::ide_commands();
        if let Some(a) = scriptide::show_palette(ctx, &mut self.ide.palette, &cmds) {
            actions.push(a);
        }
        if let Some(line) = scriptide::show_goto(ctx, &mut self.ide.goto) {
            actions.push(IdeAction::JumpLine(line));
        }

        // ---- 执行动作（统一入口）----
        for a in actions {
            self.handle_ide_action(a, ctx, editor_id);
        }

        // ---- 状态栏（VSCode 蓝条：Ln/Col、选中、缩进、换行、编码、语言）----
        let cursor = egui::TextEdit::load_state(ctx, editor_id)
            .and_then(|st| st.cursor.char_range());
        let caret_now = cursor.map(|r| r.primary.index).unwrap_or(0);
        let sel = cursor
            .map(|r| r.primary.index.abs_diff(r.secondary.index))
            .unwrap_or(0);
        let (ln, col) = scriptide::line_col_of_char(
            &self.script_text,
            caret_now.min(self.script_text.chars().count()),
        );
        let (indent_label, _) = scriptide::detect_indent(&self.script_text);
        ui.add_space(4.0);
        scriptide::status_bar(
            ui,
            &scriptide::IdeStatus {
                tab_label: &tab_label,
                line: ln,
                col,
                sel,
                indent_label,
                errors: usize::from(has_error || !inst_errs.is_empty()),
                wrap: self.ide.wrap,
                dirty: self.script_dirty,
            },
            accent,
        );

        // ---- 问题面板（VSCode Problems）：解析失败时显示首个错误与位置；
        // 解析成功时显示组件指令静态校验问题（MI 脚本 v2，步骤 33）----
        if let Err(e) = &parse_result {
            ui.add_space(4.0);
            egui::Frame::none()
                .fill(Color32::from_rgba_unmultiplied(90, 50, 20, 70))
                .rounding(8.0)
                .inner_margin(egui::Margin::symmetric(10.0, 6.0))
                .show(ui, |ui| {
                    let pos_txt = error_pos
                        .map(|(l, c)| format!("行 {l}, 列 {c}"))
                        .unwrap_or_else(|| "位置未知".to_string());
                    ui.label(
                        egui::RichText::new(format!("× 脚本解析错误（{pos_txt}）：{e}"))
                            .color(AMBER)
                            .size(12.0),
                    );
                });
        } else if !inst_errs.is_empty() {
            ui.add_space(4.0);
            egui::Frame::none()
                .fill(Color32::from_rgba_unmultiplied(90, 50, 20, 70))
                .rounding(8.0)
                .inner_margin(egui::Margin::symmetric(10.0, 6.0))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(format!("! 指令校验：{} 个问题", inst_errs.len()))
                            .color(AMBER)
                            .size(12.0),
                    );
                    for e in &inst_errs {
                        ui.label(egui::RichText::new(format!("  {e}")).color(AMBER).size(12.0));
                    }
                });
        }

        // 脏检测：脚本内容与当前配置的序列化结果不同即视为脏（配置很小，序列化开销可忽略）
        let regenerated = toml::to_string_pretty(&self.cfg).unwrap_or_default();
        self.script_dirty = !self.script_text.is_empty() && self.script_text != regenerated;
    }

    /// 编辑器标签名（项目文件名，未保存时用应用名 slug 兜底）。
    fn tab_label(&self) -> String {
        self.project_file
            .as_ref()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| {
                let slug = miedit_core::util::slugify(
                    if self.cfg.app.name.trim().is_empty() {
                        "project"
                    } else {
                        &self.cfg.app.name
                    },
                );
                format!("{slug}.mi")
            })
    }

    /// 命令面板命令表（标签, 说明, 动作）。
    fn ide_commands() -> Vec<(&'static str, &'static str, IdeAction)> {
        vec![
            ("应用修改到向导", "回填向导各页", IdeAction::Apply),
            ("保存脚本", "Ctrl+S", IdeAction::Save),
            ("另存为脚本...", "", IdeAction::SaveAs),
            ("打开项目...", "", IdeAction::OpenProject),
            ("重新生成脚本（丢弃编辑）", "", IdeAction::Regenerate),
            ("查找", "Ctrl+F", IdeAction::ToggleFind),
            ("替换", "Ctrl+H", IdeAction::ToggleReplace),
            ("转到行", "Ctrl+G", IdeAction::GotoLine),
            ("自动换行", "Alt+Z", IdeAction::ToggleWrap),
            ("转到构建页", "", IdeAction::OpenBuild),
            ("MI 脚本语法说明", "", IdeAction::HelpSyntax),
        ]
    }

    /// 执行一个 IDE 动作（菜单栏 / 工具栏 / 命令面板 / 查找小部件 / minimap 共用）。
    fn handle_ide_action(&mut self, a: IdeAction, ctx: &egui::Context, editor_id: egui::Id) {
        match a {
            IdeAction::Apply => {
                match toml::from_str::<Config>(&self.script_text) {
                    Ok(cfg) => {
                        self.cfg = cfg;
                        if self.cfg.ui.text.is_empty() {
                            self.cfg.ui.text = TEXT_DEFAULTS_ZH
                                .iter()
                                .map(|(k, v)| (k.to_string(), v.to_string()))
                                .collect();
                        }
                        self.load_background(ctx);
                        self.load_license_preview();
                        self.sync_reg_from_cfg();
                        self.refresh_files_stat();
                        self.script_dirty = false;
                        self.set_flash("脚本修改已应用到向导各页");
                    }
                    Err(e) => self.set_flash(format!("脚本解析失败: {e}")),
                }
            }
            IdeAction::Save => match self.project_file.clone() {
                Some(p) => {
                    let _ = std::fs::write(&p, self.script_text.as_str());
                    self.set_flash(format!("已保存 {}", p.display()));
                }
                None => self.save_project(),
            },
            IdeAction::SaveAs => {
                let slug = miedit_core::util::slugify(
                    if self.cfg.app.name.trim().is_empty() {
                        "project"
                    } else {
                        &self.cfg.app.name
                    },
                );
                if let Some(p) = rfd::FileDialog::new()
                    .set_file_name(format!("{slug}.mi"))
                    .add_filter("MI 脚本（.mi）", &["mi", "toml"])
                    .save_file()
                {
                    let _ = std::fs::write(&p, self.script_text.as_str());
                    self.project_file = Some(p);
                    self.set_flash("脚本已另存");
                }
            }
            IdeAction::Regenerate => {
                self.apply_reg_to_cfg();
                self.script_text = toml::to_string_pretty(&self.cfg)
                    .unwrap_or_else(|e| format!("# 序列化失败: {e}"));
                self.script_dirty = false;
            }
            IdeAction::OpenProject => {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("MI 脚本（.mi）", &["mi", "toml"])
                    .pick_file()
                {
                    if let Err(e) = self.open_project(&p, ctx) {
                        self.set_flash(format!("打开项目失败: {e:#}"));
                    }
                }
            }
            IdeAction::OpenBuild => {
                self.pending_jump = Some(Step::Build as usize);
            }
            IdeAction::ToggleFind => {
                self.ide.find.open = true;
                self.ide.find.replace_open = false;
                self.ide.find.want_focus = true;
            }
            IdeAction::ToggleReplace => {
                self.ide.find.open = true;
                self.ide.find.replace_open = true;
                self.ide.find.want_focus = true;
            }
            IdeAction::GotoLine => {
                self.ide.goto.open = true;
            }
            IdeAction::ToggleWrap => {
                self.ide.wrap = !self.ide.wrap;
            }
            IdeAction::TogglePalette => {
                self.ide.palette.open = !self.ide.palette.open;
                self.ide.palette.query.clear();
                self.ide.palette.selected = 0;
            }
            IdeAction::CaseToggle => {
                self.ide.find.case_sensitive = !self.ide.find.case_sensitive;
            }
            IdeAction::CloseFind => {
                self.ide.find.open = false;
            }
            IdeAction::FindNext | IdeAction::FindPrev => {
                let ms = scriptide::find_matches(
                    &self.script_text,
                    &self.ide.find.query,
                    self.ide.find.case_sensitive,
                );
                if ms.is_empty() {
                    self.set_flash("查找：无结果");
                } else {
                    let n = ms.len();
                    self.ide.find.current = if a == IdeAction::FindNext {
                        (self.ide.find.current + 1) % n
                    } else {
                        self.ide.find.current.checked_sub(1).unwrap_or(n - 1)
                    };
                    let (s0, e0) = ms[self.ide.find.current.min(n - 1)];
                    scriptide::select_range(ctx, editor_id, s0, e0);
                }
            }
            IdeAction::ReplaceOne => {
                let ms = scriptide::find_matches(
                    &self.script_text,
                    &self.ide.find.query,
                    self.ide.find.case_sensitive,
                );
                if ms.is_empty() {
                    self.set_flash("替换：无结果");
                } else {
                    let idx = self.ide.find.current.min(ms.len() - 1);
                    let (new_text, caret) = scriptide::replace_one(
                        &self.script_text,
                        &ms,
                        idx,
                        &self.ide.find.replacement,
                    );
                    self.script_text = new_text;
                    scriptide::set_caret(ctx, editor_id, caret);
                }
            }
            IdeAction::ReplaceAll => {
                let ms = scriptide::find_matches(
                    &self.script_text,
                    &self.ide.find.query,
                    self.ide.find.case_sensitive,
                );
                let n = ms.len();
                if n == 0 {
                    self.set_flash("替换：无结果");
                } else {
                    self.script_text = scriptide::replace_matches(
                        &self.script_text,
                        &ms,
                        &self.ide.find.replacement,
                    );
                    self.set_flash(format!("已替换 {n} 处"));
                }
            }
            IdeAction::JumpLine(line) => {
                // 选中整行并定位（TextEdit 会自动滚动到光标）
                let total = self.script_text.chars().count();
                let start = scriptide::goto_line_char(&self.script_text, line);
                let end = scriptide::goto_line_char(&self.script_text, line + 1);
                let end = if end <= start { total } else { end.saturating_sub(1) };
                scriptide::select_range(ctx, editor_id, start, end);
            }
            IdeAction::HelpSyntax => {
                self.set_flash(
                    "MI 脚本（.mi）＝ TOML 语法：[节头] 分节；键 = \"值\"；# 注释；数组 [a, b]；布尔 true/false。与向导各页完全互通。\n组件指令（v2，NSIS Section 指令语义）：组件表内 code = [{ op = \"run\" | \"write_reg\" | \"create_dir\" | \"delete_file\" | \"detail_print\", ... }] 内联数组；路径可用 $INSTDIR/$DESKTOP/$SMPROGRAMS 等 NSIS 常量变量。",
                );
            }
        }
    }

    fn page_build(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let base = ui.available_width() - 40.0;

        // Inno wpReady 风格：构建前配置摘要
        field_label(ui, "准备就绪 —— 安装配置摘要：");
        let cfg = &self.cfg;
        let (n_files, bytes) = self.files_stat;
        let comp_count = cfg.components.len();
        let comp_line = if comp_count > 0 {
            format!(
                "{} 个组件（允许用户选择：{}）",
                comp_count,
                if cfg.ui.show_components_page.unwrap_or(true) { "是" } else { "否" }
            )
        } else {
            "不使用组件（全部安装）".to_string()
        };
        let tasks_line = if cfg.install.run_tasks.is_empty() {
            "仅运行主程序（若存在）".to_string()
        } else {
            format!("{} 个任务", cfg.install.run_tasks.len())
        };
        egui::Frame::none()
            .fill(FIELD)
            .rounding(8.0)
            .stroke(egui::Stroke::new(1.0_f32, CARD_STROKE))
            .inner_margin(egui::Margin::same(12.0))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                let rows: [(&str, String); 7] = [
                    ("应用程序", format!("{} {}", cfg.app.name, cfg.app.version)),
                    ("发布者", if cfg.app.publisher.is_empty() { "—".into() } else { cfg.app.publisher.clone() }),
                    ("安装目录", cfg.install.dir_template.clone()),
                    ("文件", format!("{n_files} 个 · 未压缩 {}", miedit_core::util::human_size(bytes))),
                    ("组件", comp_line),
                    ("完成后运行", tasks_line),
                    (
                        "压缩方式",
                        match cfg.install.compression.as_str() {
                            "solid" => "Deflate Solid（整包，NSIS 风格）".into(),
                            "lzma" => "LZMA（条目级）".into(),
                            "lzma-solid" => "LZMA Solid（整包，NSIS 默认风格）".into(),
                            "brotli" => "Brotli（条目级）".into(),
                            "brotli-solid" => "Brotli Solid（整包）".into(),
                            _ => "Deflate（条目级）".into(),
                        },
                    ),
                ];
                for (k, v) in rows {
                    ui.horizontal(|ui| {
                        ui.add_sized([110.0, 0.0], egui::Label::new(egui::RichText::new(k).color(TEXT_DIM).size(12.5)));
                        ui.label(egui::RichText::new(v).color(TEXT).size(13.0));
                    });
                }
            });
        ui.add_space(8.0);

        field_label(ui, "输出安装程序：");
        ui.horizontal(|ui| {
            let out_hint = self.output_path().display().to_string();
            text_input(ui, &mut self.output, out_hint.as_str(), base - 90.0);
            if ui.add(small_button("浏览...")).clicked() {
                let default_name = self
                    .output_path()
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "app-setup.exe".into());
                if let Some(p) = rfd::FileDialog::new()
                    .set_file_name(default_name)
                    .add_filter("安装包", &["exe"])
                    .save_file()
                {
                    self.output = p.display().to_string();
                }
            }
        });
        ui.horizontal(|ui| {
            let (stub, unp) = locate_components();
            chip(ui, if stub.exists() { "安装器组件 √" } else { "安装器组件 ×" }, if stub.exists() { GREEN } else { RED });
            chip(ui, if unp.exists() { "卸载器组件 √" } else { "卸载器组件 ×" }, if unp.exists() { GREEN } else { RED });
            let (n, bytes) = self.files_stat;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format!("{n} 个文件 · {}", miedit_core::util::human_size(bytes)))
                        .color(TEXT_DIM)
                        .size(12.0),
                );
            });
        });
        ui.add_space(8.0);

        let state_snapshot = self.build.lock().unwrap().clone();
        match &state_snapshot {
            BuildState::Running => {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(18.0).color(self.accent()));
                    ui.label(egui::RichText::new("正在打包...").color(TEXT_DIM).size(13.5));
                });
            }
            _ => {
                if ui.add(primary_button("开始构建", self.accent())).clicked() {
                    self.start_build(ctx);
                }
            }
        }

        match state_snapshot {
            BuildState::Done(summary, log) => {
                ui.add_space(4.0);
                chip(ui, "构建成功", GREEN);
                ui.label(
                    egui::RichText::new(format!(
                        "{}\n总大小 {} · {} 个文件 · 压缩后 {}",
                        summary.output.display(),
                        miedit_core::util::human_size(summary.total_size),
                        summary.file_count,
                        miedit_core::util::human_size(summary.payload_len),
                    ))
                    .color(TEXT)
                    .size(12.5),
                );
                ui.horizontal(|ui| {
                    if ui.add(small_button("打开所在文件夹")).clicked() {
                        if let Some(parent) = summary.output.parent() {
                            let _ = miedit_core::winutil::shell_open(parent);
                        }
                    }
                    if ui.add(small_button("运行安装包")).clicked() {
                        let _ = miedit_core::winutil::shell_open(&summary.output);
                    }
                });
                log_lines(ui, &log);
            }
            BuildState::Failed(err) => {
                ui.add_space(4.0);
                chip(ui, "构建失败", RED);
                ui.label(egui::RichText::new(err).color(RED).size(12.5));
            }
            BuildState::Running => {
                let log = self.build_log.lock().unwrap();
                if !log.is_empty() {
                    log_lines(ui, &log);
                }
            }
            BuildState::Idle => {}
        }
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("建议先「保存项目」（页脚）将配置存为 .mi（MI 脚本），以便后续继续编辑或用 miedit-compiler 命令行编译。")
                .color(Color32::from_rgb(110, 116, 130))
                .size(11.5),
        );
    }
}

// ---------------------------------------------------------------------------
// 主体
// ---------------------------------------------------------------------------

impl BuilderApp {
    fn header(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let idx = self.step_index();
        let (title, desc) = STEP_TITLES[idx];
        ui.horizontal(|ui| {
            ui.add_space(18.0);
            ui.vertical(|ui| {
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new(format!(
                        "{title}（{}/{}）",
                        idx + 1,
                        STEP_TITLES.len()
                    ))
                        .color(TEXT)
                        .size(19.0)
                        .strong(),
                );
                if !desc.is_empty() {
                    ui.label(egui::RichText::new(desc).color(TEXT_DIM).size(12.5));
                }
                ui.add_space(8.0);
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(18.0);
                // MIEdit 徽标
                let accent = self.accent();
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
                ui.painter().rect_filled(rect, 10.0, accent.linear_multiply(0.2));
                ui.painter().rect_stroke(rect, 10.0, egui::Stroke::new(1.0_f32, accent.linear_multiply(0.7)));
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "M",
                    FontId::proportional(20.0),
                    accent,
                );
            });
        });
        ui.separator();
        let _ = ctx;
    }

    fn footer(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.separator();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(18.0);
            // 左：项目操作
            if ui.add(small_button("打开项目")).clicked() {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("MI 脚本（.mi）", &["mi", "toml"])
                    .pick_file()
                {
                    if let Err(e) = self.open_project(&p, ctx) {
                        self.set_flash(format!("打开项目失败: {e:#}"));
                    }
                }
            }
            if ui.add(small_button("保存项目")).clicked() {
                self.save_project();
            }
            // 左二：步骤提示
            let hint = match self.step {
                Step::AppInfo => "带 * 为必填项",
                Step::FilesComponents => "组件引用文件规则编号 [n]",
                _ => "",
            };
            if !hint.is_empty() {
                ui.label(egui::RichText::new(hint).color(Color32::from_rgb(110, 116, 130)).size(11.5));
            }

            // 右：导航
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(10.0);
                let close_label = if self.step == Step::Build { "退出" } else { "取消" };
                if ui.add(nav_button(close_label, None)).clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if self.step == Step::Build {
                    // 构建页：下一步隐藏
                } else {
                    let can_next = match self.step {
                        Step::AppInfo => !self.cfg.app.name.trim().is_empty(),
                        _ => true,
                    };
                    let next_label = format!("下一步(N) >");
                    if ui
                        .add_enabled(can_next, nav_button(&next_label, Some(self.accent())))
                        .clicked()
                    {
                        self.advance(ctx);
                    }
                }
                if self.step != Step::Welcome {
                    if ui.add(nav_button("上一步(B)", None)).clicked() {
                        self.step = Step::from_index(self.step_index() - 1);
                    }
                }
            });
        });
        ui.add_space(8.0);

        // 闪现提示（4 秒后消失）
        let expired = self
            .flash
            .as_ref()
            .map(|(_, t)| t.elapsed().as_secs() >= 4)
            .unwrap_or(false);
        if expired {
            self.flash = None;
        }
    }

    fn advance(&mut self, ctx: &egui::Context) {
        self.goto_step(ctx, self.step_index() + 1);
    }

    /// 跳转到指定步骤（页脚「下一步」与左栏导航共用；HM NIS Edit 向导
    /// 允许点击任意步骤直达，进入脚本页时按脏状态重新生成脚本）。
    fn goto_step(&mut self, ctx: &egui::Context, idx: usize) {
        self.apply_reg_to_cfg();
        self.step = Step::from_index(idx);
        if self.step == Step::Options {
            // 进入外观页时刷新预览纹理
            self.load_background(ctx);
        }
        if self.step == Step::UninstallReg {
            self.sync_reg_from_cfg();
        }
        if self.step == Step::Script {
            // 进入脚本页：从当前配置重新生成脚本（若有未应用的手工编辑则保留）
            if !self.script_dirty {
                self.script_text = match toml::to_string_pretty(&self.cfg) {
                    Ok(t) => t,
                    Err(e) => {
                        self.set_flash(format!("生成脚本失败: {e}"));
                        String::new()
                    }
                };
            }
        }
    }

    /// 左栏步骤导航（HM NIS Edit 式，路线图步骤 29）：全部步骤可点直达；
    /// 当前步高亮（accent 底色），已完成步带 √ 徽标，未到步暗淡。
    fn step_rail(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new("构建向导")
                .color(TEXT_DIM)
                .size(11.5)
                .strong(),
        );
        ui.add_space(6.0);
        for i in 0..STEP_SHORT.len() {
            let current = i == self.step_index();
            let done = i < self.step_index();
            let (bg, stroke) = if current {
                (RAIL_CURRENT_BG, egui::Stroke::new(1.0_f32, accent_color(&self.cfg)))
            } else {
                (Color32::TRANSPARENT, egui::Stroke::NONE)
            };
            let badge_color = if current {
                accent_color(&self.cfg)
            } else if done {
                GREEN
            } else {
                Color32::from_rgb(90, 96, 110)
            };
            let resp = egui::Frame::none()
                .fill(bg)
                .rounding(8.0)
                .stroke(stroke)
                .inner_margin(egui::Margin::symmetric(8.0, 5.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::new(20.0, 20.0), Sense::hover());
                        let p = ui.painter_at(r);
                        p.circle_filled(
                            r.center(),
                            10.0,
                            badge_color.linear_multiply(if current || done { 0.85 } else { 0.5 }),
                        );
                        let label = if done { "√".to_string() } else { format!("{}", i + 1) };
                        p.text(
                            r.center(),
                            egui::Align2::CENTER_CENTER,
                            label,
                            FontId::proportional(11.0),
                            Color32::WHITE,
                        );
                        let name_color = if current { TEXT } else { TEXT_DIM };
                        ui.label(egui::RichText::new(STEP_SHORT[i]).color(name_color).size(13.0));
                    });
                })
                .response
                .interact(egui::Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.clicked() {
                self.pending_jump = Some(i);
            }
            ui.add_space(1.0);
        }
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new("MI 脚本（.mi）与向导完全互通：
向导页改动会同步进脚本，
脚本页「应用修改」回填向导。")
                .color(Color32::from_rgb(110, 116, 130))
                .size(11.0),
        );
    }

    fn content(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        match self.step {
            Step::Welcome => self.page_welcome(ui, ctx),
            Step::AppInfo => self.page_app_info(ui, ctx),
            Step::Options => self.page_options(ui, ctx),
            Step::DirLicense => self.page_dir_license(ui, ctx),
            Step::FilesComponents => self.page_files_components(ui),
            Step::Shortcuts => self.page_shortcuts(ui),
            Step::RunAfter => self.page_run_after(ui),
            Step::UninstallReg => self.page_uninstall_reg(ui),
            Step::Script => self.page_script(ui, ctx),
            Step::Build => self.page_build(ui, ctx),
        }
    }
}

impl Step {
    fn from_index(i: usize) -> Step {
        match i {
            0 => Step::Welcome,
            1 => Step::AppInfo,
            2 => Step::Options,
            3 => Step::DirLicense,
            4 => Step::FilesComponents,
            5 => Step::Shortcuts,
            6 => Step::RunAfter,
            7 => Step::UninstallReg,
            8 => Step::Script,
            _ => Step::Build,
        }
    }
}

impl eframe::App for BuilderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // NSIS/HM NIS Edit 操作方式：Enter 触发下一步/开始构建，Esc 取消
        // （脚本编辑页不拦截，多行编辑器需要 Enter/Escape）
        if self.step != Step::Script {
            let (enter, esc) = ctx.input(|i| {
                (
                    i.key_pressed(egui::Key::Enter),
                    i.key_pressed(egui::Key::Escape),
                )
            });
            if esc {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            if enter {
                let can_next = match self.step {
                    Step::AppInfo => !self.cfg.app.name.trim().is_empty(),
                    _ => true,
                };
                if self.step == Step::Build {
                    self.start_build(ctx);
                } else if can_next {
                    self.apply_reg_to_cfg();
                    self.step = Step::from_index(self.step_index() + 1);
                    if self.step == Step::Options {
                        self.load_background(ctx);
                    }
                    if self.step == Step::UninstallReg {
                        self.sync_reg_from_cfg();
                    }
                    if self.step == Step::Script {
                        self.apply_reg_to_cfg();
                        self.script_text = toml::to_string_pretty(&self.cfg)
                            .unwrap_or_else(|e| format!("# 序列化失败: {e}"));
                        self.script_dirty = false;
                    }
                }
            }
        }

        // 顶栏：标题 + 徽标
        egui::TopBottomPanel::top("wizard-header")
            .frame(egui::Frame::none().fill(BG_TOP))
            .show(ctx, |ui| {
                self.header(ui, ctx);
            });

        // 底栏：导航
        egui::TopBottomPanel::bottom("wizard-footer")
            .frame(egui::Frame::none().fill(BG_TOP))
            .show(ctx, |ui| {
                self.footer(ui, ctx);
            });

        // 左栏步骤导航（HM NIS Edit 式，路线图步骤 29）
        egui::SidePanel::left("wizard-rail")
            .resizable(false)
            .exact_width(188.0)
            .frame(egui::Frame::none().fill(BG_TOP))
            .show(ctx, |ui| {
                self.step_rail(ui);
            });

        // 左栏点击跳转（rail 中只记录，此处执行以避免借用冲突）
        if let Some(i) = self.pending_jump.take() {
            if i != self.step_index() {
                self.goto_step(ctx, i);
            }
        }

        // 中间内容
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(BG_BOTTOM)
                    .inner_margin(egui::Margin {
                        left: 18.0,
                        right: 18.0,
                        top: 10.0,
                        bottom: 10.0,
                    }),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("wizard-content")
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        // 闪现提示
                        if let Some((msg, t)) = &self.flash {
                            if t.elapsed().as_secs() < 4 {
                                chip(ui, msg, AMBER);
                            }
                        }
                        self.content(ui, ctx);
                        ui.add_space(20.0);
                    });
            });
    }
}
