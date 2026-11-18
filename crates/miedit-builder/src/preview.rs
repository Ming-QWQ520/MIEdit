//! 安装界面实时预览：按当前配置（背景图/主题色/文案/窗口尺寸）等比缩小渲染欢迎页。

use crate::{accent_color, CARD, TEXT, TEXT_DIM};
use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, Sense, TextureHandle, Vec2,
};
use miedit_core::config::{fill_template, ui_text, Config};

/// 在给定区域内绘制安装窗口的等比预览。
/// `avail_w` 由调用方传入（基于面板宽度的稳定值，避免布局反馈）。
pub fn draw(ui: &mut egui::Ui, tex: Option<&TextureHandle>, cfg: &Config, avail_w: f32) {
    let accent = accent_color(cfg);
    let max_h = 460.0;
    let [w, h] = cfg.ui.window_size;
    let s = (avail_w / w).min(max_h / h).clamp(0.18, 1.0);
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
                let c = Color32::from_rgb(f(top.r(), bottom.r()), f(top.g(), bottom.g()), f(top.b(), bottom.b()));
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
    p.text(
        Pos2::new(rect.left() + s_(18.0), rect.top() + s_(22.0)),
        Align2::LEFT_CENTER,
        fill_template(&ui_text(cfg, "window_title"), &cfg.app.name, &cfg.app.version),
        FontId::proportional(s_(13.5)),
        TEXT_DIM,
    );
    // 关闭 / 最小化
    let cx = Pos2::new(rect.right() - s_(26.0), rect.top() + s_(22.0));
    let stroke = egui::Stroke::new(1.6 * s.max(0.6), TEXT_DIM);
    p.line_segment([cx - Vec2::new(s_(5.0), s_(5.0)), cx + Vec2::new(s_(5.0), s_(5.0))], stroke);
    p.line_segment([cx - Vec2::new(-s_(5.0), s_(5.0)), cx + Vec2::new(-s_(5.0), s_(5.0))], stroke);
    let mx = Pos2::new(rect.right() - s_(64.0), rect.top() + s_(22.0));
    p.line_segment(
        [Pos2::new(mx.x - s_(5.0), mx.y), Pos2::new(mx.x + s_(5.0), mx.y)],
        stroke,
    );

    // ---- 卡片 ----
    let card = Rect::from_min_max(
        Pos2::new(rect.left() + s_(44.0), rect.top() + s_(108.0)),
        Pos2::new(rect.right() - s_(44.0), rect.bottom() - s_(104.0)),
    );
    p.rect_filled(card, s_(18.0), CARD.linear_multiply(0.62));
    p.rect(card, s_(18.0), Color32::TRANSPARENT, egui::Stroke::new(1.0_f32, Color32::from_rgb(52, 56, 66)));

    // ---- 欢迎内容 ----
    let inner = Rect::from_min_max(
        Pos2::new(card.left() + s_(28.0), card.top() + s_(22.0)),
        Pos2::new(card.right() - s_(28.0), card.bottom() - s_(22.0)),
    );
    let mut y = inner.top() + s_(6.0);

    // chip
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

    // ---- 底部按钮 ----
    let by = rect.bottom() - s_(32.0) - s_(21.0);
    let bx = rect.right() - s_(44.0);
    // 下一步（主按钮）
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
    // 取消（次按钮）
    let cancel_w = s_(118.0);
    p.rect_filled(
        Rect::from_min_size(Pos2::new(bx - next_w - s_(12.0) - cancel_w, by), Vec2::new(cancel_w, next_h)),
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
