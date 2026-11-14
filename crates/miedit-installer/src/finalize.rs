//! 安装收尾：写入卸载器、创建快捷方式、注册表卸载信息、卸载清单。

use crate::install::{push_log, ProgressState};
use crate::pkg::Package;
use anyhow::{Context as _, Result};
use miedit_core::config::ui_text;
use miedit_core::manifest::UninstallManifest;
use miedit_core::winutil::{self, RegValue};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;

pub fn finalize_install(
    pkg: &Package,
    dest: &Path,
    files: &[String],
    dirs: &[String],
    st: &Mutex<ProgressState>,
) -> Result<UninstallManifest> {
    let cfg = &pkg.config;
    let per_machine = cfg.app.per_machine;
    let app_id = cfg.app_id_or_slug();

    // 1. 释放卸载器
    push_log(st, ui_text(cfg, "finalizing"));
    let mut f = fs::File::open(&pkg.path).context("打开安装包失败")?;
    f.seek(SeekFrom::Start(pkg.trailer.uninstaller.off))?;
    let mut un_bytes = vec![0u8; pkg.trailer.uninstaller.len as usize];
    f.read_exact(&mut un_bytes)?;
    let uninst_path = dest.join("uninstall.exe");
    fs::write(&uninst_path, &un_bytes).context("写入卸载器失败")?;

    let exe_abs = if cfg.app.exe.is_empty() {
        None
    } else {
        Some(dest.join(&cfg.app.exe))
    };

    let mut manifest = UninstallManifest {
        app_name: cfg.app.name.clone(),
        app_id: app_id.clone(),
        version: cfg.app.version.clone(),
        publisher: cfg.app.publisher.clone(),
        install_dir: dest.display().to_string(),
        per_machine,
        files: files.to_vec(),
        dirs: dirs.to_vec(),
        shortcuts: Vec::new(),
        shortcut_dirs: Vec::new(),
        registry_keys: Vec::new(),
        uninstall_key: format!(
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{app_id}"
        ),
        // 主程序相对路径（卸载前运行检测用，NSIS un.onInit / Inno CloseApplications 对齐）
        main_exe: cfg.app.exe.clone(),
        user_data_dirs: cfg
            .install
            .user_data_dirs
            .iter()
            .map(|tpl| {
                miedit_core::util::resolve_path_template(tpl, &cfg.app.name)
                    .display()
                    .to_string()
            })
            .collect(),
        uninstaller_path: uninst_path.display().to_string(),
        manifest_path: String::new(),
        installed_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    };

    // 2. 快捷方式
    let sc_name = cfg
        .shortcuts
        .name
        .clone()
        .unwrap_or_else(|| cfg.app.name.clone());
    if cfg.shortcuts.start_menu {
        let dir = winutil::start_menu_dir(per_machine).join(&sc_name);
        fs::create_dir_all(&dir).context("创建开始菜单目录失败")?;
        manifest.shortcut_dirs.push(dir.display().to_string());
        if let Some(exe) = &exe_abs {
            let lnk = dir.join(format!("{sc_name}.lnk"));
            winutil::create_shortcut(&lnk, exe, &cfg.shortcuts.run_args, dest, Some(exe), &cfg.app.name)?;
            manifest.shortcuts.push(lnk.display().to_string());
        }
        let lnk_un = dir.join(format!("卸载 {sc_name}.lnk"));
        winutil::create_shortcut(&lnk_un, &uninst_path, "", dest, None, &format!("卸载 {}", cfg.app.name))?;
        manifest.shortcuts.push(lnk_un.display().to_string());
        push_log(st, format!("开始菜单快捷方式: {}", dir.display()));
    }
    if cfg.shortcuts.desktop {
        let d = winutil::desktop_dir(per_machine);
        fs::create_dir_all(&d).context("创建桌面目录失败")?;
        if let Some(exe) = &exe_abs {
            let lnk = d.join(format!("{sc_name}.lnk"));
            winutil::create_shortcut(&lnk, exe, &cfg.shortcuts.run_args, dest, Some(exe), &cfg.app.name)?;
            manifest.shortcuts.push(lnk.display().to_string());
            push_log(st, format!("桌面快捷方式: {}", lnk.display()));
        }
    }

    // 3. 注册表：标准卸载信息（控制面板可见）
    push_log(st, "写入注册表卸载信息...");
    let total_kb = ((pkg.total_bytes + un_bytes.len() as u64) / 1024).max(1) as u32;
    let mut values: Vec<(String, RegValue)> = vec![
        ("DisplayName".into(), RegValue::Str(cfg.app.name.clone())),
        ("DisplayVersion".into(), RegValue::Str(cfg.app.version.clone())),
        ("Publisher".into(), RegValue::Str(cfg.app.publisher.clone())),
        ("InstallLocation".into(), RegValue::Str(dest.display().to_string())),
        (
            "UninstallString".into(),
            RegValue::Str(format!("\"{}\"", uninst_path.display())),
        ),
        ("NoModify".into(), RegValue::Dword(1)),
        ("NoRepair".into(), RegValue::Dword(1)),
        ("EstimatedSize".into(), RegValue::Dword(total_kb)),
        // NSIS 对齐：静默卸载入口（Add/Remove Programs 自动化使用）
        (
            "QuietUninstallString".into(),
            RegValue::Str(format!("\"{}\" /S", uninst_path.display())),
        ),
    ];
    if let Some(exe) = &exe_abs {
        values.push(("DisplayIcon".into(), RegValue::Str(exe.display().to_string())));
    }
    if !cfg.app.url.trim().is_empty() {
        values.push(("URLInfoAbout".into(), RegValue::Str(cfg.app.url.trim().to_string())));
    }
    winutil::registry_create_set(per_machine, &manifest.uninstall_key, &values)?;

    // 4. 脚本声明的自定义注册表值
    for spec in &cfg.registry {
        if spec.key.trim().is_empty() {
            continue;
        }
        let install_dir_str = dest.display().to_string();
        let resolved: Vec<(String, RegValue)> = spec
            .values
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    RegValue::Str(v.replace("{install_dir}", &install_dir_str)),
                )
            })
            .collect();
        let (pm, rest) = winutil::split_hive(&spec.key)?;
        winutil::registry_create_set(pm, &rest, &resolved)?;
        manifest.registry_keys.push(spec.key.clone());
        push_log(st, format!("注册表键: {}", spec.key));
    }

    // 5. 卸载清单内嵌进卸载器 EXE 尾部（NSIS 式自包含卸载器，不再落盘 uninstall.json）
    let json = serde_json::to_vec_pretty(&manifest)?;
    // 卸载器图标：优先使用内嵌的 uninstall_icon 分区，否则复用应用图标
    let mut icon_bytes: Vec<u8> = Vec::new();
    let icon_range = if !pkg.trailer.uninstall_icon.is_empty() {
        pkg.trailer.uninstall_icon
    } else {
        pkg.trailer.icon
    };
    if !icon_range.is_empty() {
        let mut f2 = fs::File::open(&pkg.path)?;
        f2.seek(SeekFrom::Start(icon_range.off))?;
        let mut ib = vec![0u8; icon_range.len as usize];
        f2.read_exact(&mut ib)?;
        icon_bytes = ib;
    }
    miedit_core::trailer::write_uninstall_data(&uninst_path, &json, &icon_bytes)?;
    manifest.manifest_path = String::new();

    push_log(
        st,
        format!(
            "已写入 {} 个文件、{} 个目录、{} 个快捷方式",
            files.len(),
            dirs.len(),
            manifest.shortcuts.len()
        ),
    );
    Ok(manifest)
}
