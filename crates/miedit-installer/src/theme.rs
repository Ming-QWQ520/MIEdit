//! 主题与视觉：字体（中文字体回退）、样式、背景绘制、按钮样式。

use crate::pkg::Package;
use eframe::egui::{self, Color32, FontId, Pos2, Rect, Sense, Stroke, TextureHandle, Vec2};

pub struct Theme {
    pub accent: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub card: Color32,
    pub card_stroke: Color32,
    pub field: Color32,
}

impl Theme {
    pub fn from_config(pkg: &Package) -> Self {
        let [r, g, b] = miedit_core::util::parse_hex_color(&pkg.config.ui.accent)
            .unwrap_or([0x3B, 0x82, 0xF6]);
        let accent = Color32::from_rgb(r, g, b);
        Self {
            accent,
            text: Color32::from_rgb(240, 242, 247),
            text_dim: Color32::from_rgb(198, 204, 216),
            card: Color32::from_rgb(10, 12, 18),
            card_stroke: Color32::from_rgb(52, 56, 66),
            field: Color32::from_rgb(8, 10, 14),
        }
    }

    pub fn apply(&self, ctx: &egui::Context) {
        ctx.set_visuals(egui::Visuals::dark());
        let accent = self.accent;
        let text = self.text;
        ctx.style_mut(|st| {
            let v = &mut st.visuals;
            v.panel_fill = Color32::TRANSPARENT;
            v.window_fill = Color32::TRANSPARENT;
            v.extreme_bg_color = Color32::from_rgb(6, 8, 12);
            v.faint_bg_color = Color32::from_rgb(18, 21, 28);
            v.selection.bg_fill = accent;
            v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
            v.widgets.noninteractive.bg_stroke = Stroke::NONE;
            v.widgets.inactive.bg_fill = Color32::from_rgb(20, 23, 30);
            v.widgets.inactive.bg_stroke = Stroke::NONE;
            v.widgets.inactive.fg_stroke = Stroke::new(1.0, text);
            v.widgets.hovered.bg_fill = Color32::from_rgb(30, 34, 44);
            v.widgets.active.bg_fill = accent;
            v.window_rounding = egui::Rounding::same(12.0);
            st.spacing.item_spacing = Vec2::new(10.0, 10.0);
            st.spacing.button_padding = Vec2::new(16.0, 9.0);
            st.spacing.scroll = egui::style::ScrollStyle::solid();
        });
    }
}

/// 安装中文字体（优先微软雅黑，其次黑体/宋体），作为缺字回退。
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
            fonts
                .font_data
                .insert(name.clone(), egui::FontData::from_owned(data));
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .push(name.clone());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push(name);
            break;
        }
    }
    ctx.set_fonts(fonts);
}

/// 绘制全窗口背景：图片（cover/contain/stretch）或程序化深色渐变。
/// 必须在面板内最先调用（同层按插入顺序渲染，保证在最底层）。
pub fn paint_background(
    painter: &egui::Painter,
    sr: Rect,
    tex: Option<&TextureHandle>,
    style: &str,
    accent: Color32,
) {
    match tex {
        Some(t) => {
            if style == "contain" {
                painter.rect_filled(sr, 0.0, Color32::from_rgb(9, 11, 17));
                let r = contain_rect(t.size_vec2(), sr.size());
                painter.image(
                    t.id(),
                    r,
                    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                let uv = match style {
                    "stretch" => Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                    _ => cover_uv(t.size_vec2(), sr.size()),
                };
                painter.image(t.id(), sr, uv, Color32::WHITE);
            }
            // 轻度压暗保证前景可读
            painter.rect_filled(sr, 0.0, Color32::from_rgb(10, 12, 20).linear_multiply(0.25));
        }
        None => {
            // 程序化背景：深蓝夜空渐变 + 右上主题色光斑
            let top = Color32::from_rgb(13, 17, 31);
            let bottom = Color32::from_rgb(8, 10, 16);
            let n = 48;
            for i in 0..n {
                let t = (i as f32 + 0.5) / n as f32;
                let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
                let c = Color32::from_rgb(f(top.r(), bottom.r()), f(top.g(), bottom.g()), f(top.b(), bottom.b()));
                let y0 = sr.top() + sr.height() * (i as f32 / n as f32);
                let y1 = sr.top() + sr.height() * ((i + 1) as f32 / n as f32);
                painter.rect_filled(
                    Rect::from_min_max(Pos2::new(sr.left(), y0), Pos2::new(sr.right(), y1)),
                    0.0,
                    c,
                );
            }
            for i in 0..6 {
                let r = 320.0 - i as f32 * 48.0;
                painter.circle_filled(
                    Pos2::new(sr.right() - 90.0, sr.top() + 130.0),
                    r,
                    Color32::from_rgb(
                        accent.r(),
                        accent.g(),
                        accent.b(),
                    )
                    .linear_multiply(0.10 - i as f32 * 0.014),
                );
            }
        }
    }
}

/// 计算保持宽高比覆盖目标区域时的 UV 裁剪。
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

/// contain 模式下图片应绘制的窗口内矩形。
fn contain_rect(img: Vec2, win: Vec2) -> Rect {
    if img.x <= 0.0 || img.y <= 0.0 {
        return Rect::from_min_size(Pos2::new(0.0, 0.0), win);
    }
    let s = (win.x / img.x).min(win.y / img.y);
    let size = img * s;
    let min = Pos2::new((win.x - size.x) / 2.0, (win.y - size.y) / 2.0);
    Rect::from_min_size(min, size)
}

pub fn primary_button<'a>(text: &'a str, t: &'a Theme) -> egui::Button<'a> {
    egui::Button::new(
        egui::RichText::new(text)
            .color(Color32::WHITE)
            .size(15.5)
            .strong(),
    )
    .fill(t.accent)
    .rounding(10.0)
    .min_size(Vec2::new(118.0, 42.0))
}

pub fn secondary_button<'a>(text: &'a str, t: &'a Theme) -> egui::Button<'a> {
    egui::Button::new(egui::RichText::new(text).color(t.text_dim).size(15.5))
        .fill(Color32::from_rgb(18, 21, 28))
        .stroke(Stroke::new(1.0, t.card_stroke))
        .rounding(10.0)
        .min_size(Vec2::new(118.0, 42.0))
}

/// 底部细进度条（自绘）。
pub fn draw_progress(ui: &mut egui::Ui, frac: f32, accent: Color32, height: f32) {
    let avail = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(avail, height), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, height * 0.5, Color32::from_rgb(12, 14, 20));
    let w = rect.width() * frac.clamp(0.0, 1.0);
    if w > height * 0.5 {
        painter.rect_filled(
            Rect::from_min_max(rect.min, Pos2::new(rect.left() + w, rect.bottom())),
            height * 0.5,
            accent,
        );
    }
}

/// 圆形徽标（完成/警告）。
pub fn draw_badge(ui: &mut egui::Ui, color: Color32, glyph: &str, radius: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(radius * 2.0), Sense::hover());
    let painter = ui.painter();
    let c = rect.center();
    painter.circle_filled(c, radius, color.linear_multiply(0.22));
    painter.circle_filled(c, radius * 0.82, color);
    painter.text(
        c,
        egui::Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(radius * 0.95),
        Color32::WHITE,
    );
}
