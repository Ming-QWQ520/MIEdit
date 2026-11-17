//! MIEdit 卸载器：读取安装目录中的 uninstall.json，执行清理并自删除。
//!
//! GUI 页面：确认 → 卸载中 → 完成/失败。`--silent` 时无界面直接执行。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::{anyhow, Context, Result};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2, ViewportCommand};
use miedit_core::manifest::UninstallManifest;
use miedit_core::winutil;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const ACCENT: Color32 = Color32::from_rgb(0xE5, 0x48, 0x4D);
const GREEN: Color32 = Color32::from_rgb(0x3D, 0xD6, 0x8C);
const AMBER: Color32 = Color32::from_rgb(0xF5, 0xA6, 0x23);
const BG_TOP: Color32 = Color32::from_rgb(0x17, 0x1A, 0x21);
const BG_BOTTOM: Color32 = Color32::from_rgb(0x0D, 0x0E, 0x12);
const CARD: Color32 = Color32::from_black_alpha(110);
const CARD_STROKE: Color32 = Color32::from_rgb(43, 46, 54); // 近似 from_white_alpha(20) 叠加深底
const TEXT: Color32 = Color32::from_rgb(238, 240, 245);
const TEXT_DIM: Color32 = Color32::from_rgb(185, 191, 203);

#[derive(Debug, Clone, PartialEq)]
enum Phase {
    Confirm,
    Working,
    /// 卸载完成详情停留页（NSIS INSTFILES 语义，路线图步骤 18）：
    /// 显示完成状态 + 全部日志，用户点「下一步」进入完成总结页
    Summary,
    Done,
    Failed(String),
}

#[derive(Default)]
struct RunState {
    current: String,
    done_count: usize,
    total: usize,
    log: Vec<String>,
    finished: bool,
    errors: Vec<String>,
    /// 已计划重启后删除的文件数（NSIS Delete /REBOOTOK）
    reboot_scheduled: usize,
    /// 卸载统计（NSIS 卸载完成统计对齐，路线图步骤 14）：已删除文件数/目录数/释放字节数
    files_removed: usize,
    dirs_removed: usize,
    bytes_freed: u64,
}

struct UninstallerApp {
    manifest: UninstallManifest,
    run: Arc<Mutex<RunState>>,
    phase: Phase,
    /// 是否同时删除用户数据/配置目录
    delete_user_data: bool,
    /// 应用图标纹理（内嵌于卸载器，标题栏与确认页展示）
    icon_tex: Option<egui::TextureHandle>,
    /// 卸载器壁纸纹理（内嵌于卸载器，步骤 19；None = 内置渐变背景）
    bg_tex: Option<egui::TextureHandle>,
    /// 卸载页“显示细节”开关（默认收起）
    show_details: bool,
    /// 主程序是否正在运行（确认页警告，可重新检测）
    app_running: bool,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let silent = args.iter().any(|a| {
        let l = a.to_ascii_lowercase();
        l == "--silent" || l == "/s" || l == "-s"
    });
    match real_main(silent, &args) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            winutil::message_box_error("卸载程序", &format!("{e:#}"));
            std::process::exit(1);
        }
    }
}

fn real_main(silent: bool, args: &[String]) -> Result<i32> {
    let exe = std::env::current_exe().context("获取自身路径失败")?;
    let dir = exe
        .parent()
        .ok_or_else(|| anyhow!("无法定位卸载器所在目录"))?
        .to_path_buf();

    // NSIS 式自包含卸载器：优先读取自身尾部的内嵌清单（MIEDUNIN，含图标与壁纸）；
    // 兼容旧版：24B footer（无壁纸）回退，最旧版回退到 uninstall.json
    let (manifest, icon_bytes, bg_bytes): (UninstallManifest, Option<Vec<u8>>, Option<Vec<u8>>) =
        match miedit_core::trailer::read_uninstall_data(&exe) {
            Some((m, icon, bg)) if !m.is_empty() => (
                serde_json::from_slice(&m).context("内嵌卸载清单损坏")?,
                Some(icon),
                if bg.is_empty() { None } else { Some(bg) },
            ),
            _ => {
                let manifest_path = dir.join("uninstall.json");
                let m: UninstallManifest = serde_json::from_slice(
                    &std::fs::read(&manifest_path).context(
                        "未找到卸载清单（内嵌或 uninstall.json）—— 可能已被卸载或安装不完整",
                    )?,
                )
                .context("卸载清单损坏")?;
                (m, None, None)
            }
        };

    if manifest.per_machine && !winutil::is_admin() {
        if winutil::relaunch_elevated(args)? {
            return Ok(0);
        }
        anyhow::bail!("本次安装需要管理员权限才能卸载");
    }

    let run = Arc::new(Mutex::new(RunState::default()));

    if silent {
        // 卸载前运行检测（NSIS un.onInit / Inno CloseApplications 对齐）：
        // 静默模式记录警告后继续，被占用文件转入重启删除
        if main_exe_running(&manifest) {
            run.lock().unwrap().log.push(
                format!("警告: 主程序 {} 可能正在运行，被占用的文件将在重启后删除", manifest.main_exe),
            );
        }
        let errors = do_uninstall(&manifest, &run, None, false);
        let mut log = run.lock().unwrap().log.clone();
        let reboot = run.lock().unwrap().reboot_scheduled;
        if reboot > 0 {
            log.push(format!("注意: {reboot} 个文件将在系统重启后删除"));
        }
        write_silent_log(&log);
        if errors.is_empty() {
            cleanup_after_uninstall(&manifest);
            Ok(0)
        } else {
            eprintln!("卸载完成，但有 {} 个错误（详见日志）", errors.len());
            cleanup_after_uninstall(&manifest);
            Ok(1)
        }
    } else {
        let app_running = main_exe_running(&manifest);
        run_gui(manifest, icon_bytes, bg_bytes, run, app_running)
    }
}

/// 主程序是否正在运行（清单携带 main_exe 时检测文件占用）。
fn main_exe_running(m: &UninstallManifest) -> bool {
    if m.main_exe.trim().is_empty() {
        return false;
    }
    winutil::is_file_locked(&PathBuf::from(&m.install_dir).join(&m.main_exe))
}

fn write_silent_log(log: &[String]) {
    if let Ok(tmp) = std::env::var("TEMP") {
        let p = PathBuf::from(tmp).join("miedit-uninstall.log");
        let _ = std::fs::write(&p, log.join("\n"));
    }
}

// ---------------------------------------------------------------------------
// 卸载执行
// ---------------------------------------------------------------------------

fn do_uninstall(
    m: &UninstallManifest,
    run: &Mutex<RunState>,
    ctx: Option<&egui::Context>,
    delete_user_data: bool,
) -> Vec<String> {
    let user_data = if delete_user_data { &m.user_data_dirs } else { &Vec::new() };
    // 步骤 22：注册表操作数（新清单用精确清理条数，旧清单回退键数）
    let reg_ops = if m.registry_values.is_empty() {
        m.registry_keys.len()
    } else {
        m.registry_values.len()
    };
    let total = m.files.len() + m.dirs.len() + m.shortcuts.len() + m.shortcut_dirs.len()
        + reg_ops + user_data.len() + 1;
    {
        let mut r = run.lock().unwrap();
        r.total = total;
        r.log.push(format!("开始卸载 {}", m.app_name));
    }

    let mut errors: Vec<String> = Vec::new();
    let tick = |run: &Mutex<RunState>, current: String, ctx: Option<&egui::Context>| {
        let mut r = run.lock().unwrap();
        r.current = current;
        r.done_count += 1;
        drop(r);
        if let Some(c) = ctx {
            c.request_repaint();
        }
    };

    // 0. 卸载前命令（步骤 23：NSIS 卸载脚本 Exec 语义；如停服务/关闭进程）。
    //    失败不中止卸载（继续删除，被占用文件走重试/重启删除路径），仅记录错误
    for (i, spec) in m.exec_before_uninstall.iter().enumerate() {
        let describe = format!("卸载前命令 #{}", i + 1);
        tick(run, format!("执行{describe}: {}", spec.cmd.trim()), ctx);
        let outcome = miedit_core::runcmd::execute(spec, Path::new(&m.install_dir));
        for line in miedit_core::runcmd::log_lines(&describe, spec, &outcome) {
            run.lock().unwrap().log.push(line);
        }
        if miedit_core::runcmd::should_abort(spec, &outcome) {
            errors.push(format!(
                "{} 失败: {}（已忽略，继续卸载）",
                describe,
                miedit_core::runcmd::abort_reason(spec, &outcome)
            ));
        }
    }

    // 1. 文件（NSIS Delete /REBOOTOK 对齐：占用中的文件转入重启删除，不算失败）
    for f in &m.files {
        let p = PathBuf::from(f);
        // 统计释放空间（尽力而为；删除失败则不计入）
        let sz = std::fs::metadata(&p).map(|md| md.len()).unwrap_or(0);
        if let Err(e) = winutil::remove_file_quiet(&p) {
            match winutil::schedule_delete_on_reboot(&p) {
                Ok(()) => {
                    let mut r = run.lock().unwrap();
                    r.reboot_scheduled += 1;
                    r.log
                        .push(format!("已计划重启后删除（文件占用）: {}", p.display()));
                    r.current = format!("重启删除 {}", p.display());
                    r.done_count += 1;
                    drop(r);
                    if let Some(c) = ctx {
                        c.request_repaint();
                    }
                }
                Err(re) => errors.push(format!("删除失败: {e:#}；且无法计划重启删除: {re:#}")),
            }
        } else {
            {
                let mut r = run.lock().unwrap();
                r.files_removed += 1;
                r.bytes_freed += sz;
            }
            tick(run, format!("删除 {}", p.display()), ctx);
        }
    }

    // 2. 目录（最深的先删）
    let mut dirs: Vec<&String> = m.dirs.iter().collect();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.split('/').count()));
    for d in dirs {
        let p = m.install_dir.trim_end_matches(['\\', '/']).to_string() + "\\" + d;
        let removed = match winutil::remove_dir_quiet(Path::new(&p)) {
            Ok(()) => true,
            Err(e) => {
                errors.push(format!("{e:#}"));
                false
            }
        };
        if removed {
            let mut r = run.lock().unwrap();
            r.dirs_removed += 1;
        }
        tick(run, format!("移除目录 {d}"), ctx);
    }

    // 3. 快捷方式与快捷方式目录（最深先删，子目录嵌套时避免父目录先删失败）
    for s in &m.shortcuts {
        let p = Path::new(s);
        let sz = std::fs::metadata(p).map(|md| md.len()).unwrap_or(0);
        if winutil::remove_file_quiet(p).is_ok() {
            let mut r = run.lock().unwrap();
            r.files_removed += 1;
            r.bytes_freed += sz;
        } else {
            errors.push(format!("删除快捷方式失败: {s}"));
        }
        tick(run, format!("移除快捷方式 {s}"), ctx);
    }
    let mut sdirs: Vec<&String> = m.shortcut_dirs.iter().collect();
    sdirs.sort_by_key(|d| std::cmp::Reverse(d.split(['\\', '/']).count()));
    for sd in sdirs {
        if winutil::remove_dir_quiet(Path::new(sd)).is_ok() {
            let mut r = run.lock().unwrap();
            r.dirs_removed += 1;
        } else {
            errors.push(format!("删除快捷方式目录失败: {sd}"));
        }
        tick(run, format!("移除目录 {sd}"), ctx);
    }

    // 4. 用户数据/配置目录（可选，勾选后才删除）
    if !user_data.is_empty() {
        let mut ud: Vec<&String> = user_data.iter().collect();
        ud.sort_by_key(|d| std::cmp::Reverse(d.split(['\\', '/']).count()));
        for d in ud {
            let p = PathBuf::from(d);
            // 删除前递归统计（文件数/字节数，NSIS 卸载统计对齐）
            let (uf, ub) = miedit_core::util::dir_file_stats(&p);
            if let Err(e) = winutil::remove_dir_all_quiet(&p) {
                errors.push(format!("{e:#}"));
            } else {
                let mut r = run.lock().unwrap();
                r.files_removed += uf;
                r.bytes_freed += ub;
                r.dirs_removed += 1;
            }
            tick(run, format!("删除用户数据 {}", p.display()), ctx);
        }
    }

    // 5. 注册表（步骤 22 精确清理：本次新建的键整树删除，既有键只删本次写入的
    //    值并尝试清理空壳键——不误伤安装前已存在的内容；旧清单无 registry_values
    //    字段 → 回退整树删除 registry_keys，行为与旧版卸载器一致）
    if !m.registry_values.is_empty() {
        for r in &m.registry_values {
            if let Ok((pm, rest)) = winutil::split_hive(&r.key) {
                if r.created_key {
                    if let Err(e) = winutil::registry_delete_tree(pm, &rest) {
                        errors.push(format!("{e:#}"));
                    }
                } else {
                    if !r.name.is_empty() {
                        if let Err(e) = winutil::registry_delete_value(pm, &rest, &r.name, winutil::RegView::Default) {
                            errors.push(format!("{e:#}"));
                        }
                    }
                    if let Err(e) = winutil::registry_delete_subkey_if_empty(pm, &rest, winutil::RegView::Default) {
                        errors.push(format!("{e:#}"));
                    }
                }
            }
            tick(run, format!("移除注册表值 {}", r.key), ctx);
        }
    } else {
        for k in &m.registry_keys {
            if let Ok((per_machine, rest)) = winutil::split_hive(k) {
                if let Err(e) = winutil::registry_delete_tree(per_machine, &rest) {
                    errors.push(format!("{e:#}"));
                }
            }
            tick(run, format!("移除注册表键 {k}"), ctx);
        }
    }
    if let Err(e) = winutil::registry_delete_tree(m.per_machine, &m.uninstall_key) {
        errors.push(format!("{e:#}"));
    }
    tick(run, "移除卸载注册表信息".into(), ctx);

    let mut r = run.lock().unwrap();
    r.finished = true;
    // 卸载统计（NSIS 卸载完成统计对齐，路线图步骤 14）
    let (fr, dr, bf) = (r.files_removed, r.dirs_removed, r.bytes_freed);
    r.log.push(format!(
        "已删除 {fr} 个文件、{dr} 个目录，释放 {}",
        miedit_core::util::human_size(bf)
    ));
    if errors.is_empty() {
        r.log.push("卸载完成".into());
    } else {
        r.log.push(format!("卸载完成，{} 项未能删除", errors.len()));
        for e in &errors {
            r.log.push(format!("  · {e}"));
        }
        r.errors = errors.clone();
    }
    drop(r);
    if let Some(c) = ctx {
        c.request_repaint();
    }
    errors
}

/// 关闭后清理：删除清单文件、自删除卸载器、尝试移除空的安装目录。
fn cleanup_after_uninstall(m: &UninstallManifest) {
    let _ = std::fs::remove_file(&m.manifest_path);
    winutil::schedule_self_delete(
        Path::new(&m.uninstaller_path),
        &[PathBuf::from(&m.install_dir)],
    );
}

// ---------------------------------------------------------------------------
// GUI
// ---------------------------------------------------------------------------

fn install_fonts(ctx: &egui::Context) {
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

fn apply_style(ctx: &egui::Context) {
    ctx.set_visuals(egui::Visuals::dark());
    ctx.style_mut(|st| {
        let v = &mut st.visuals;
        v.panel_fill = Color32::TRANSPARENT;
        v.window_fill = Color32::TRANSPARENT;
        v.extreme_bg_color = Color32::from_black_alpha(120);
        v.faint_bg_color = Color32::from_black_alpha(50);
        v.selection.bg_fill = ACCENT;
        v.widgets.inactive.bg_fill = Color32::from_black_alpha(90);
        v.widgets.inactive.bg_stroke = Stroke::NONE;
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        v.widgets.hovered.bg_fill = Color32::from_black_alpha(140);
        v.widgets.active.bg_fill = Color32::from_black_alpha(180);
        st.spacing.item_spacing = Vec2::new(10.0, 10.0);
        st.spacing.button_padding = Vec2::new(14.0, 8.0);
    });
}

fn primary_button(text: &str, fill: Color32) -> egui::Button<'_> {
    egui::Button::new(egui::RichText::new(text).color(Color32::WHITE).size(15.0).strong())
        .fill(fill)
        .rounding(10.0)
        .min_size(Vec2::new(112.0, 40.0))
}

fn secondary_button(text: &str) -> egui::Button<'_> {
    egui::Button::new(egui::RichText::new(text).color(TEXT_DIM).size(15.0))
        .fill(Color32::from_black_alpha(110))
        .rounding(10.0)
        .min_size(Vec2::new(112.0, 40.0))
}

fn run_gui(
    manifest: UninstallManifest,
    icon_bytes: Option<Vec<u8>>,
    bg_bytes: Option<Vec<u8>>,
    run: Arc<Mutex<RunState>>,
    app_running: bool,
) -> Result<i32> {
    // 图标：任务栏 + 标题栏 + 确认页
    let icon_data: Option<egui::IconData> = icon_bytes.and_then(|b| decode_icon(&b));
    // 壁纸（步骤 19）：解码为 egui 纹理；有壁纸时窗口放大以充分展示
    let bg_image: Option<egui::ColorImage> = bg_bytes.and_then(|b| decode_image(&b));
    let win_size = if bg_image.is_some() { [800.0, 540.0] } else { [560.0, 420.0] };
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(format!("卸载 {}", manifest.app_name))
        .with_inner_size(win_size)
        .with_resizable(false)
        .with_decorations(false);
    if let Some(ic) = &icon_data {
        viewport = viewport.with_icon(ic.clone());
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let app = UninstallerApp {
        manifest,
        run,
        phase: Phase::Confirm,
        delete_user_data: false,
        icon_tex: None,
        bg_tex: None,
        show_details: false,
        app_running,
    };
    eframe::run_native(
        "MIEdit Uninstaller",
        options,
        Box::new(move |cc| {
            install_fonts(&cc.egui_ctx);
            apply_style(&cc.egui_ctx);
            let icon_tex = icon_data.as_ref().map(|ic| {
                cc.egui_ctx.load_texture(
                    "uninstall-icon",
                    egui::ColorImage::from_rgba_unmultiplied(
                        [ic.width as usize, ic.height as usize],
                        &ic.rgba,
                    ),
                    egui::TextureOptions::LINEAR,
                )
            });
            let bg_tex = bg_image.as_ref().map(|img| {
                cc.egui_ctx
                    .load_texture("uninstall-bg", img.clone(), egui::TextureOptions::LINEAR)
            });
            let mut app = app;
            app.icon_tex = icon_tex;
            app.bg_tex = bg_tex;
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow!("GUI 运行失败: {e}"))?;
    Ok(0)
}

/// 解码图标字节为 IconData（.ico 取最大帧；PNG 等直接解码）。
fn decode_icon(bytes: &[u8]) -> Option<egui::IconData> {
    let img = image::load_from_memory(bytes).ok()?;
    let rgba = img.to_rgba8();
    Some(egui::IconData {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.as_raw().clone(),
    })
}

/// 解码壁纸字节为 egui ColorImage（PNG/JPEG 等）。
fn decode_image(bytes: &[u8]) -> Option<egui::ColorImage> {
    let img = image::load_from_memory(bytes).ok()?;
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [img.width() as usize, img.height() as usize],
        img.to_rgba8().as_raw(),
    ))
}

impl UninstallerApp {
    /// 开始卸载（确认页「卸载」按钮 / Enter 的公共入口）
    fn start_uninstall(&mut self, ctx: &egui::Context) {
        self.phase = Phase::Working;
        let run = self.run.clone();
        let manifest = self.manifest.clone();
        let ctx2 = ctx.clone();
        let del = self.delete_user_data;
        std::thread::spawn(move || {
            do_uninstall(&manifest, &run, Some(&ctx2), del);
        });
    }

    /// 主操作（NSIS：Enter 触发默认按钮 / Esc 取消）
    fn primary_action(&mut self, ctx: &egui::Context) {
        match self.phase {
            Phase::Confirm => self.start_uninstall(ctx),
            // 步骤 18：详情停留页「下一步」→ 完成总结页（NSIS INSTFILES → FINISH）
            Phase::Summary => self.phase = Phase::Done,
            Phase::Done | Phase::Failed(_) => {
                cleanup_after_uninstall(&self.manifest);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Phase::Working => {}
        }
    }
    fn paint_background(&self, painter: &egui::Painter, sr: Rect) {
        // 步骤 19：卸载器壁纸——全窗口铺满 + 半透明遮罩保证文字可读
        if let Some(bg) = &self.bg_tex {
            painter.image(
                bg.id(),
                sr,
                Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
            painter.rect_filled(sr, 0.0, Color32::from_black_alpha(120));
            // 顶部品牌色细线 + 右上柔和光斑与渐变模式保持一致
            painter.rect_filled(
                Rect::from_min_max(sr.min, Pos2::new(sr.right(), sr.top() + 3.0)),
                0.0,
                ACCENT.linear_multiply(0.9),
            );
            for i in 0..3 {
                let r = 220.0 - i as f32 * 55.0;
                painter.circle_filled(
                    Pos2::new(sr.right() - 60.0, sr.top() + 90.0),
                    r,
                    Color32::from_rgba_unmultiplied(0xE5, 0x48, 0x4D, (8 - i * 2).max(2)),
                );
            }
            return;
        }
        let n = 48;
        for i in 0..n {
            let t = (i as f32 + 0.5) / n as f32;
            let c = lerp_color(BG_TOP, BG_BOTTOM, t);
            let y0 = sr.top() + sr.height() * (i as f32 / n as f32);
            let y1 = sr.top() + sr.height() * ((i + 1) as f32 / n as f32);
            painter.rect_filled(Rect::from_min_max(Pos2::new(sr.left(), y0), Pos2::new(sr.right(), y1)), 0.0, c);
        }
        // 顶部品牌色细线
        painter.rect_filled(
            Rect::from_min_max(sr.min, Pos2::new(sr.right(), sr.top() + 3.0)),
            0.0,
            ACCENT.linear_multiply(0.9),
        );
        // 右上柔和光斑
        for i in 0..5 {
            let r = 260.0 - i as f32 * 45.0;
            painter.circle_filled(
                Pos2::new(sr.right() - 60.0, sr.top() + 90.0),
                r,
                Color32::from_rgba_unmultiplied(0xE5, 0x48, 0x4D, (9 - i * 1).max(2)),
            );
        }
    }

    fn content(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        match self.phase.clone() {
            Phase::Confirm => self.page_confirm(ui),
            Phase::Working => self.page_working(ui),
            Phase::Summary => self.page_summary(ui),
            Phase::Done => self.page_done(ui),
            Phase::Failed(err) => self.page_failed(ui, &err),
        }
    }

    /// 步骤 18：卸载完成详情停留页（NSIS INSTFILES 完成态语义）——
    /// 完成徽标 + 统计 + 全量日志（默认展开），点「下一步」进入完成总结页。
    fn page_summary(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(4.0);
            draw_badge(ui, GREEN, "√");
            ui.add_space(4.0);
            ui.heading(
                egui::RichText::new(format!("已成功卸载 {}", self.manifest.app_name))
                    .color(TEXT)
                    .size(24.0)
                    .strong(),
            );
            let (fr, dr, bf, reboot) = {
                let r = self.run.lock().unwrap();
                (r.files_removed, r.dirs_removed, r.bytes_freed, r.reboot_scheduled)
            };
            ui.label(
                egui::RichText::new(format!(
                    "已删除 {} 个文件、{} 个目录，释放 {}",
                    fr,
                    dr,
                    miedit_core::util::human_size(bf)
                ))
                .color(TEXT_DIM)
                .size(13.5),
            );
            if reboot > 0 {
                ui.label(
                    egui::RichText::new(format!(
                        "{reboot} 个文件将在系统重启后删除。"
                    ))
                    .color(AMBER)
                    .size(13.0),
                );
            }
        });
        ui.add_space(6.0);
        egui::Frame::none()
            .fill(Color32::from_rgb(8, 10, 14))
            .rounding(10.0)
            .stroke(egui::Stroke::new(1.0, CARD_STROKE))
            .inner_margin(egui::Margin::same(10.0))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        let log = self.run.lock().unwrap().log.clone();
                        for line in &log {
                            ui.label(
                                egui::RichText::new(line)
                                    .color(TEXT_DIM)
                                    .size(12.0)
                                    .monospace(),
                            );
                        }
                    });
            });
    }

    fn page_confirm(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(8.0);
            if let Some(icon) = &self.icon_tex {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(48.0), Sense::hover());
                ui.painter().image(
                    icon.id(),
                    rect,
                    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
                ui.add_space(6.0);
            } else {
                draw_badge(ui, ACCENT, "!");
            }
            ui.add_space(6.0);
            ui.heading(egui::RichText::new(format!("卸载 {}", self.manifest.app_name)).color(TEXT).size(26.0).strong());
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!(
                    "这将从计算机中移除 {} {} 及其所有组件。\n安装目录：{}",
                    self.manifest.app_name, self.manifest.version, self.manifest.install_dir
                ))
                .color(TEXT_DIM)
                .size(14.5),
            );
            // 卸载前运行检测（NSIS un.onInit / Inno CloseApplications 对齐）
            if self.app_running {
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(format!(
                        "检测到 {} 可能正在运行，建议关闭后再卸载；\n被占用的文件将在系统重启后删除。",
                        self.manifest.app_name
                    ))
                    .color(AMBER)
                    .size(13.5),
                );
                if ui.add(secondary_button("重新检测")).clicked() {
                    self.app_running = main_exe_running(&self.manifest);
                }
            }
        });
        if !self.manifest.user_data_dirs.is_empty() {
            ui.vertical_centered(|ui| {
                ui.checkbox(
                    &mut self.delete_user_data,
                    egui::RichText::new("同时删除用户数据和配置")
                        .color(TEXT_DIM)
                        .size(13.5),
                );
            });
        }
    }

    fn page_working(&mut self, ui: &mut egui::Ui) {
        let r = self.run.lock().unwrap();
        ui.add_space(6.0);
        ui.heading(egui::RichText::new(format!("正在卸载 {}", self.manifest.app_name)).color(TEXT).size(26.0).strong());
        ui.add_space(10.0);
        let frac = if r.total == 0 { 1.0 } else { r.done_count as f32 / r.total as f32 };
        draw_progress(ui, frac, ACCENT);
        ui.add_space(4.0);
        ui.label(egui::RichText::new(&r.current).color(TEXT_DIM).size(13.0));
        ui.add_space(8.0);
        // NSIS “Show details” 风格：默认收起，可展开查看日志
        let show = self.show_details;
        ui.horizontal(|ui| {
            let label = if show { "隐藏细节" } else { "显示细节" };
            if ui.add(small_toggle_button(label)).clicked() {
                self.show_details = !self.show_details;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format!("{}/{} 项", r.done_count, r.total))
                        .color(TEXT_DIM)
                        .size(12.0),
                );
            });
        });
        if show {
            egui::Frame::none()
                .fill(Color32::from_rgb(8, 10, 14))
                .rounding(10.0)
                .stroke(egui::Stroke::new(1.0, CARD_STROKE))
                .inner_margin(egui::Margin::same(10.0))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink(false)
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            for line in &r.log {
                                ui.label(
                                    egui::RichText::new(line)
                                        .color(TEXT_DIM)
                                        .size(12.0)
                                        .monospace(),
                                );
                            }
                        });
                });
        }
    }

    fn page_done(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(8.0);
            draw_badge(ui, GREEN, "√");
            ui.add_space(6.0);
            ui.heading(egui::RichText::new(format!("已成功卸载 {}", self.manifest.app_name)).color(TEXT).size(26.0).strong());
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("关闭窗口后，卸载程序将自动清理自身文件。")
                    .color(TEXT_DIM)
                    .size(14.0),
            );
            // 卸载统计（NSIS 卸载完成统计对齐，路线图步骤 14）
            let (fr, dr, bf) = {
                let r = self.run.lock().unwrap();
                (r.files_removed, r.dirs_removed, r.bytes_freed)
            };
            ui.label(
                egui::RichText::new(format!(
                    "已删除 {} 个文件、{} 个目录，释放 {}",
                    fr,
                    dr,
                    miedit_core::util::human_size(bf)
                ))
                .color(TEXT_DIM)
                .size(13.5),
            );
            // NSIS /REBOOTOK 对齐：占用文件已计划重启删除
            let reboot = self.run.lock().unwrap().reboot_scheduled;
            if reboot > 0 {
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(format!(
                        "{reboot} 个文件正在使用中，已计划在系统重启后删除；重启后将完成全部清理。"
                    ))
                    .color(AMBER)
                    .size(13.5),
                );
            }
        });
    }

    fn page_failed(&mut self, ui: &mut egui::Ui, err: &str) {
        ui.vertical_centered(|ui| {
            draw_badge(ui, ACCENT, "!");
            ui.add_space(4.0);
            ui.heading(egui::RichText::new("卸载遇到问题").color(TEXT).size(26.0).strong());
            ui.add_space(4.0);
            ui.label(egui::RichText::new(err).color(TEXT_DIM).size(13.5));
            ui.add_space(8.0);
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                let r = self.run.lock().unwrap();
                for line in &r.log {
                    ui.label(egui::RichText::new(line).color(TEXT_DIM).size(12.0).monospace());
                }
            });
        });
    }
}

#[allow(deprecated)]
impl eframe::App for UninstallerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // NSIS 操作方式：Enter 触发主操作，Esc 取消（卸载进行中忽略）
        let (enter, esc) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if esc && self.phase != Phase::Working {
            cleanup_after_uninstall(&self.manifest);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if enter && self.phase != Phase::Working {
            self.primary_action(ctx);
        }

        // 工作线程完成检测（步骤 18：无错误 → Summary 详情停留页，用户点「下一步」进完成页）
        if self.phase == Phase::Working {
            let finished = self.run.lock().unwrap().finished;
            if finished {
                let has_errors = !self.run.lock().unwrap().errors.is_empty();
                if has_errors {
                    let errs = self.run.lock().unwrap().errors.clone();
                    self.phase = Phase::Failed(errs.join("\n"));
                } else {
                    self.phase = Phase::Summary;
                }
            }
        }

        egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| {
            let sr = ui.max_rect();
            self.paint_background(ui.painter(), sr);

            // 无边框标题栏：拖拽 + 关闭
            let tb = Rect::from_min_size(sr.min, Vec2::new(sr.width(), 42.0));
            let drag = ui.interact(tb, egui::Id::new("drag"), Sense::click_and_drag());
            if drag.dragged() {
                ctx.send_viewport_cmd(ViewportCommand::StartDrag);
            }
            // 标题栏应用图标（内嵌 uninstall.ico）
            let mut title_x = sr.left() + 16.0;
            if let Some(icon) = &self.icon_tex {
                let icon_rect = Rect::from_min_size(
                    Pos2::new(sr.left() + 14.0, sr.top() + 10.0),
                    Vec2::splat(20.0),
                );
                ui.painter().image(
                    icon.id(),
                    icon_rect,
                    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
                title_x = sr.left() + 42.0;
            }
            ui.painter().text(
                Pos2::new(title_x, sr.top() + 21.0),
                Align2::LEFT_CENTER,
                format!("卸载 {}", self.manifest.app_name),
                FontId::proportional(13.5),
                TEXT_DIM,
            );
            let close_rect = Rect::from_min_size(Pos2::new(sr.right() - 40.0, sr.top() + 7.0), Vec2::new(34.0, 28.0));
            let crect = ui.interact(close_rect, egui::Id::new("win-close"), Sense::click());
            if crect.hovered() {
                ui.painter().rect_filled(close_rect, 7.0, Color32::from_white_alpha(26));
            }
            let c = close_rect.center();
            let stroke = Stroke::new(1.6, TEXT_DIM);
            ui.painter().line_segment([c - Vec2::new(5.0, 5.0), c + Vec2::new(5.0, 5.0)], stroke);
            ui.painter().line_segment([c - Vec2::new(-5.0, 5.0), c + Vec2::new(-5.0, 5.0)], stroke);
            if crect.clicked() {
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }

            // 卡片
            let card = Rect::from_min_max(
                Pos2::new(sr.left() + 28.0, sr.top() + 64.0),
                Pos2::new(sr.right() - 28.0, sr.bottom() - 88.0),
            );
            ui.painter().rect_filled(card, 16.0, CARD);
            ui.painter().rect(card, 16.0, Color32::TRANSPARENT, Stroke::new(1.0, CARD_STROKE));

            ui.allocate_ui_at_rect(card.shrink2(Vec2::new(22.0, 18.0)), |ui| {
                self.content(ui, ctx);
            });

            // 底部按钮
            let phase = self.phase.clone();
            egui::Area::new(egui::Id::new("buttons"))
                .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-28.0, -24.0))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        match phase {
                            Phase::Confirm => {
                                if ui.add(secondary_button("取消")).clicked() {
                                    cleanup_after_uninstall(&self.manifest);
                                    ctx.send_viewport_cmd(ViewportCommand::Close);
                                }
                                if ui.add(primary_button("卸载", ACCENT)).clicked() {
                                    self.start_uninstall(ctx);
                                }
                            }
                            Phase::Working => {}
                            Phase::Summary => {
                                if ui.add(secondary_button("取消")).clicked() {
                                    cleanup_after_uninstall(&self.manifest);
                                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                }
                                if ui.add(primary_button("下一步", GREEN)).clicked() {
                                    self.primary_action(ctx);
                                }
                            }
                            Phase::Done | Phase::Failed(_) => {
                                let label = if matches!(phase, Phase::Done) { "完成" } else { "关闭" };
                                if ui.add(primary_button(label, if matches!(phase, Phase::Done) { GREEN } else { ACCENT })).clicked() {
                                    self.primary_action(ctx);
                                }
                            }
                        }
                    });
                });
        });
    }
}

// ---------------------------------------------------------------------------
// 绘制小部件
// ---------------------------------------------------------------------------

fn small_toggle_button<'a>(text: &'a str) -> egui::Button<'a> {
    egui::Button::new(egui::RichText::new(text).color(TEXT_DIM).size(12.5))
        .fill(Color32::from_black_alpha(110))
        .stroke(egui::Stroke::new(1.0, CARD_STROKE))
        .rounding(8.0)
        .min_size(Vec2::new(0.0, 28.0))
}

fn draw_badge(ui: &mut egui::Ui, color: Color32, glyph: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(64.0, 64.0), Sense::hover());
    let painter = ui.painter();
    let c = rect.center();
    painter.circle_filled(c, 32.0, color.linear_multiply(0.22));
    painter.circle_filled(c, 26.0, color);
    painter.text(
        c,
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(30.0),
        Color32::WHITE,
    );
}

fn draw_progress(ui: &mut egui::Ui, frac: f32, accent: Color32) {
    let avail = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(avail, 8.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 4.0, Color32::from_black_alpha(140));
    let w = (rect.width() * frac.clamp(0.0, 1.0)).max(0.0);
    if w > 0.5 {
        painter.rect_filled(
            Rect::from_min_max(rect.min, Pos2::new(rect.left() + w, rect.bottom())),
            4.0,
            accent,
        );
    }
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}
