//! 安装收尾：写入卸载器、创建快捷方式、注册表卸载信息、卸载清单。

use crate::install::{push_log, ProgressState};
use crate::pkg::Package;
use anyhow::{Context as _, Result};
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
    language: Option<usize>,
) -> Result<UninstallManifest> {
    let cfg = &pkg.config;
    let per_machine = cfg.app.per_machine;
    let app_id = cfg.app_id_or_slug();

    // 1. 释放卸载器（文案按选中语言解析，步骤 15 多语言）
    push_log(
        st,
        miedit_core::config::resolved_text(cfg, language, "finalizing"),
    );
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

    // 2. 快捷方式（简单开关 + Inno [Icons] 风格条目，路线图步骤 13）
    // created_dirs 去重：同一目录只记录一次（卸载时清理），桌面/快速启动栏等系统目录不记录
    let sc_name = cfg
        .shortcuts
        .name
        .clone()
        .unwrap_or_else(|| cfg.app.name.clone());
    let mut created_dirs: std::collections::BTreeSet<std::path::PathBuf> =
        std::collections::BTreeSet::new();
    if cfg.shortcuts.start_menu {
        let dir = winutil::start_menu_dir(per_machine).join(&sc_name);
        fs::create_dir_all(&dir).context("创建开始菜单目录失败")?;
        created_dirs.insert(dir.clone());
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

    // 2.5 快捷方式条目（Inno [Icons] 对齐，路线图步骤 13）：
    // 自定义名称/子目录/参数/图标/工作目录/描述，位置可多选
    // （start_menu / desktop / quick_launch）；单个条目失败不阻断安装。
    for e in cfg.shortcuts.entries.iter() {
        let name = e.name.trim();
        // 目标：空 = 主程序；目标不存在时警告并跳过（不阻断安装）
        let target = if e.exe.trim().is_empty() {
            match &exe_abs {
                Some(p) => p.clone(),
                None => {
                    push_log(
                        st,
                        format!("警告: 快捷方式 \"{name}\" 未指定 exe 且应用无主程序，已跳过"),
                    );
                    continue;
                }
            }
        } else {
            let p = dest.join(e.exe.trim());
            if !p.exists() {
                push_log(
                    st,
                    format!(
                        "警告: 快捷方式 \"{name}\" 目标不存在，已跳过: {}（安装后再创建的条目请指向已安装文件）",
                        p.display()
                    ),
                );
                continue;
            }
            p
        };
        // 工作目录：空 = 目标所在目录；支持 {install_dir} 模板
        let workdir = {
            let w = e.working_dir.trim().replace("{install_dir}", &dest.display().to_string());
            let p = std::path::PathBuf::from(w);
            if p.is_absolute() {
                p
            } else {
                dest.join(p)
            }
        };
        // 图标：包内相对路径（安装后文件），缺失时回退目标自身图标
        let icon = if e.icon.trim().is_empty() {
            None
        } else {
            let p = dest.join(e.icon.trim());
            if p.exists() { Some(p) } else { None }
        };
        let desc = if e.comment.trim().is_empty() {
            cfg.app.name.clone()
        } else {
            e.comment.trim().to_string()
        };
        for loc in &e.locations {
            let (base, use_sub) = match loc.trim().to_ascii_lowercase().as_str() {
                "start_menu" => (winutil::start_menu_dir(per_machine), true),
                "desktop" => (winutil::desktop_dir(per_machine), false),
                "quick_launch" => (winutil::quick_launch_dir(), false),
                _ => {
                    push_log(st, format!("警告: 快捷方式 \"{name}\" 未知位置 \"{loc}\"，已跳过"));
                    continue;
                }
            };
            // 子目录仅对开始菜单生效（Inno {group} 语义）；空 = 应用名分组目录
            let dir = if use_sub && !e.subfolder.trim().is_empty() {
                base.join(e.subfolder.trim())
            } else if use_sub {
                base.join(&sc_name)
            } else {
                base.clone()
            };
            if use_sub {
                if created_dirs.insert(dir.clone()) {
                    let _ = fs::create_dir_all(&dir);
                    manifest.shortcut_dirs.push(dir.display().to_string());
                }
            } else {
                let _ = fs::create_dir_all(&dir);
            }
            let lnk = dir.join(format!("{name}.lnk"));
            match winutil::create_shortcut(&lnk, &target, &e.args, &workdir, icon.as_deref(), &desc) {
                Ok(()) => {
                    manifest.shortcuts.push(lnk.display().to_string());
                    push_log(st, format!("快捷方式: {}", lnk.display()));
                }
                Err(err) => {
                    push_log(st, format!("警告: 快捷方式创建失败（{}）: {err:#}", lnk.display()));
                }
            }
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
        // 步骤 17（Inno ModifyPath 对齐）：写修复入口，替代旧的 NoModify=1 ——
        // 控制面板「更改/修复」按钮执行安装器 --repair，全量重装文件并重建
        // 快捷方式/注册表。安装器路径异常时回退指向卸载器（保守降级）；
        // 保留 NoRepair=1（无独立修复按钮语义，「更改」即修复入口）。
        (
            "ModifyPath".into(),
            RegValue::Str(miedit_core::repair::modify_path_value(
                &pkg.path.display().to_string(),
            )
            .unwrap_or_else(|| uninst_path.display().to_string())),
        ),
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
    //    卸载器壁纸（步骤 19）随清单/图标一起内嵌；无壁纸时空字节（32B footer bg_len=0）
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
    // 卸载器壁纸：内嵌 uninstall_bg 分区（v4 包；旧格式包无此分区 → 空 → 卸载器走内置渐变）
    let mut ubg_bytes: Vec<u8> = Vec::new();
    if !pkg.trailer.uninstall_bg.is_empty() {
        let mut f3 = fs::File::open(&pkg.path)?;
        f3.seek(SeekFrom::Start(pkg.trailer.uninstall_bg.off))?;
        let mut bb = vec![0u8; pkg.trailer.uninstall_bg.len as usize];
        f3.read_exact(&mut bb)?;
        ubg_bytes = bb;
    }
    miedit_core::trailer::write_uninstall_data(&uninst_path, &json, &icon_bytes, &ubg_bytes)?;
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
