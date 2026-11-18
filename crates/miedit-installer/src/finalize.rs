//! 安装收尾：写入卸载器、创建快捷方式、注册表卸载信息、卸载清单。
//!
//! 平台差异（步骤 21，Linux 适配）：
//! - Windows：卸载器 PE 写出为 `uninstall.exe`，卸载清单内嵌其尾部
//!   （MIEDUNIN footer，NSIS 式自包含卸载器）；
//! - Linux：包内卸载器分区为 Windows PE，不写出；卸载清单落盘为
//!   `<安装目录>/uninstall.json`，由独立发布的 `miedit-uninstaller`
//!   （同目录或 PATH）读取执行卸载（卸载器自带清单文件回退读取）；
//!   快捷方式为 freedesktop `.desktop` 条目（`winutil::create_shortcut`
//!   内部归一扩展名），注册表写入为平台桩（no-op）。

use crate::install::{push_log, ProgressState};
use crate::pkg::Package;
use anyhow::{Context as _, Result};
use miedit_core::manifest::UninstallManifest;
use miedit_core::winutil::{self, RegValue};
use std::fs;
// io 读取（包内卸载器/图标/壁纸分区）仅 Windows 内嵌路径使用
#[cfg(windows)]
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 快捷方式清单记录路径（Windows = .lnk；Linux = 实际写出的 .desktop）。
fn shortcut_record_path(lnk: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        lnk.to_path_buf()
    }
    #[cfg(not(windows))]
    {
        lnk.with_extension("desktop")
    }
}

/// MI 脚本 v2 组件指令的副作用收集（步骤 33）：安装期执行
/// `run_component_code` 时登记，`finalize_install` 合并进卸载清单，
/// 保证指令写入的注册表/目录在卸载时被精确清理（NSIS UninstallLog 对齐）。
#[derive(Debug, Default)]
pub struct CodeEffects {
    /// 指令写入的注册表键（含 hive 前缀；去重后合并）
    pub registry_keys: Vec<String>,
    /// 指令写入的注册表值引用（精确清理）
    pub registry_values: Vec<miedit_core::manifest::RegistryValueRef>,
    /// 指令在安装目录之外创建的目录（绝对路径；NSIS RMDir 语义清理）
    pub extra_dirs: Vec<String>,
}

pub fn finalize_install(
    pkg: &Package,
    dest: &Path,
    files: &[String],
    dirs: &[String],
    st: &Mutex<ProgressState>,
    language: Option<usize>,
    shortcuts_enabled: bool,
    code_effects: &CodeEffects,
) -> Result<UninstallManifest> {
    let cfg = &pkg.config;
    let per_machine = cfg.app.per_machine;
    let app_id = cfg.app_id_or_slug();

    // 1. 释放卸载器（文案按选中语言解析，步骤 15 多语言）
    push_log(
        st,
        miedit_core::config::resolved_text(cfg, language, "finalizing"),
    );
    #[cfg(windows)]
    let un_bytes: Vec<u8> = {
        let mut f = fs::File::open(&pkg.path).context("打开安装包失败")?;
        f.seek(SeekFrom::Start(pkg.trailer.uninstaller.off))?;
        let mut b = vec![0u8; pkg.trailer.uninstaller.len as usize];
        f.read_exact(&mut b)?;
        b
    };
    #[cfg(not(windows))]
    let un_bytes: Vec<u8> = Vec::new();
    // 包内卸载器为 Windows PE；Linux 上不写出（由 uninstall.json 清单承担）
    let uninst_path = dest.join("uninstall.exe");
    #[cfg(windows)]
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
        registry_values: Vec::new(),
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
        uninstaller_path: {
            #[cfg(windows)]
            {
                uninst_path.display().to_string()
            }
            #[cfg(not(windows))]
            {
                String::new()
            }
        },
        manifest_path: String::new(),
        installed_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        // 卸载前命令随清单内嵌/落盘（步骤 23；卸载器无脚本，只读清单）
        exec_before_uninstall: cfg.install.exec_before_uninstall.clone(),
        // 语言包副本随清单内嵌/落盘（步骤 26，NSIS un MUI 对齐）
        languages: cfg.ui.languages.clone(),
        // 安装时选定的语言 id（步骤 28，NSIS MUI 语义：卸载器跟随安装语言）。
        // 单语言包安装器跳过语言页（initial_language 预选 Some(0)）→ 同样记录；
        // 无语言包（None）→ 留空，卸载器回退内置 zh-CN（旧包行为不变）
        language: language
            .and_then(|i| cfg.ui.languages.get(i))
            .map(|l| l.id.trim().to_string())
            .unwrap_or_default(),
        // 环境变量清理引用（步骤 27，下方逐条填充）
        env_values: Vec::new(),
        // 组件指令在安装目录外创建的目录（步骤 33，下方合并）
        extra_dirs: Vec::new(),
    };

    // 1b. 合并组件指令副作用（步骤 33）：write_reg → 精确清理引用；
    //     create_dir（安装目录外）→ extra_dirs。去重合并避免重复清理。
    for k in &code_effects.registry_keys {
        if !manifest.registry_keys.contains(k) {
            manifest.registry_keys.push(k.clone());
        }
    }
    for v in &code_effects.registry_values {
        if !manifest.registry_values.contains(v) {
            manifest.registry_values.push(v.clone());
        }
    }
    for d in &code_effects.extra_dirs {
        if !manifest.extra_dirs.contains(d) {
            manifest.extra_dirs.push(d.clone());
        }
    }

    // 2. 快捷方式（简单开关 + Inno [Icons] 风格条目，路线图步骤 13；
    //    步骤 21：CLI --no-shortcuts 可整体跳过）
    // created_dirs 去重：同一目录只记录一次（卸载时清理），桌面/快速启动栏等系统目录不记录
    let sc_name = cfg
        .shortcuts
        .name
        .clone()
        .unwrap_or_else(|| cfg.app.name.clone());
    let mut created_dirs: std::collections::BTreeSet<std::path::PathBuf> =
        std::collections::BTreeSet::new();
    if shortcuts_enabled {
        if cfg.shortcuts.start_menu {
            let dir = winutil::start_menu_dir(per_machine).join(&sc_name);
            fs::create_dir_all(&dir).context("创建开始菜单目录失败")?;
            created_dirs.insert(dir.clone());
            manifest.shortcut_dirs.push(dir.display().to_string());
            if let Some(exe) = &exe_abs {
                let lnk = dir.join(format!("{sc_name}.lnk"));
                winutil::create_shortcut(
                    &lnk,
                    exe,
                    &cfg.shortcuts.run_args,
                    dest,
                    Some(exe),
                    &cfg.app.name,
                )?;
                manifest
                    .shortcuts
                    .push(shortcut_record_path(&lnk).display().to_string());
            }
            // 卸载快捷方式：目标为包内 Windows PE 卸载器，仅 Windows 创建
            #[cfg(windows)]
            {
                let lnk_un = dir.join(format!("卸载 {sc_name}.lnk"));
                winutil::create_shortcut(
                    &lnk_un,
                    &uninst_path,
                    "",
                    dest,
                    None,
                    &format!("卸载 {}", cfg.app.name),
                )?;
                manifest.shortcuts.push(lnk_un.display().to_string());
            }
            push_log(st, format!("开始菜单快捷方式: {}", dir.display()));
        }
        if cfg.shortcuts.desktop {
            let d = winutil::desktop_dir(per_machine);
            fs::create_dir_all(&d).context("创建桌面目录失败")?;
            if let Some(exe) = &exe_abs {
                let lnk = d.join(format!("{sc_name}.lnk"));
                winutil::create_shortcut(
                    &lnk,
                    exe,
                    &cfg.shortcuts.run_args,
                    dest,
                    Some(exe),
                    &cfg.app.name,
                )?;
                manifest
                    .shortcuts
                    .push(shortcut_record_path(&lnk).display().to_string());
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
                let w = e
                    .working_dir
                    .trim()
                    .replace("{install_dir}", &dest.display().to_string());
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
                match winutil::create_shortcut(&lnk, &target, &e.args, &workdir, icon.as_deref(), &desc)
                {
                    Ok(()) => {
                        manifest
                            .shortcuts
                            .push(shortcut_record_path(&lnk).display().to_string());
                        push_log(
                            st,
                            format!("快捷方式: {}", shortcut_record_path(&lnk).display()),
                        );
                    }
                    Err(err) => {
                        push_log(st, format!("警告: 快捷方式创建失败（{}）: {err:#}", lnk.display()));
                    }
                }
            }
        }
    } else {
        push_log(st, "已按要求跳过快捷方式创建（--no-shortcuts）".to_string());
    }

    // 3. 注册表：标准卸载信息（控制面板可见；Linux 为平台桩 no-op）
    #[cfg(windows)]
    push_log(st, "写入注册表卸载信息...");
    #[cfg(not(windows))]
    push_log(st, "非 Windows 平台：跳过注册表（卸载信息记录在清单中）");
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

    // 4. 脚本声明的自定义注册表值（步骤 22：全类型 + 32/64 视图 + 精确清理跟踪）
    for spec in &cfg.registry {
        if spec.key.trim().is_empty() {
            continue;
        }
        let install_dir_str = dest.display().to_string();
        let (resolved, view) = match (|| -> Result<_, String> {
            let v = miedit_core::regspec::resolve_entries(spec, &install_dir_str)?;
            let view = miedit_core::regspec::view_of(spec)?;
            Ok((v, view))
        })() {
            Ok(v) => v,
            Err(e) => {
                push_log(st, format!("警告: 注册表项跳过: {}（{e}）", spec.key));
                continue;
            }
        };
        let (pm, rest) = match winutil::split_hive(&spec.key) {
            Ok(v) => v,
            Err(e) => {
                push_log(st, format!("警告: 注册表项跳过: {e:#}"));
                continue;
            }
        };
        // 键是否为本次新建（精确清理：新建键卸载时可整树删除，
        // 既有键只删本次写入的值，不误伤安装前已存在的内容）
        let created_key = !winutil::registry_key_exists(pm, &rest, view);
        winutil::registry_create_set_view(pm, &rest, &resolved, view)?;
        manifest.registry_keys.push(spec.key.clone());
        if resolved.is_empty() {
            manifest.registry_values.push(miedit_core::manifest::RegistryValueRef {
                key: spec.key.clone(),
                name: String::new(),
                created_key,
            });
        } else {
            for (name, _) in &resolved {
                manifest.registry_values.push(miedit_core::manifest::RegistryValueRef {
                    key: spec.key.clone(),
                    name: name.clone(),
                    created_key,
                });
            }
        }
        push_log(
            st,
            format!(
                "注册表键: {}（{} 个值，视图 {}，{}）",
                spec.key,
                resolved.len(),
                miedit_core::regspec::view_label(view),
                if created_key { "新建键" } else { "既有键" }
            ),
        );
    }

    // 4.5 环境变量（步骤 27，NSIS WriteEnvStr 惯例 / Inno ChangesEnvironment 对齐）：
    //     `all` 直接设值（记录原值供卸载还原）；`path_append`/`path_prepend` 按
    //     `;` 列表去重合并（记录写入段供卸载时移除）。写完统一广播
    //     WM_SETTINGCHANGE，新启动进程立即可见。非 Windows 平台为桩跳过。
    #[cfg(windows)]
    {
        let install_dir_str = dest.display().to_string();
        let mut env_dirty = false;
        for (i, spec) in cfg.install.env.iter().enumerate() {
            let entry = match miedit_core::envspec::resolve_entry(spec, &install_dir_str, &cfg.app.name) {
                Ok(e) => e,
                Err(e) => {
                    push_log(st, format!("警告: 环境变量 #{} 跳过: {e}", i + 1));
                    continue;
                }
            };
            let scope_label = if entry.machine { "机器级" } else { "用户级" };
            if entry.machine && !winutil::is_admin() {
                push_log(
                    st,
                    format!("警告: 环境变量 {}（机器级）需要管理员权限，已跳过", entry.name),
                );
                continue;
            }
            match entry.part {
                miedit_core::envspec::EnvPart::All => {
                    let existed = winutil::env_read(entry.machine, &entry.name);
                    if let Err(e) = winutil::env_write(entry.machine, &entry.name, &entry.value) {
                        push_log(st, format!("警告: 环境变量 {} 写入失败: {e:#}", entry.name));
                        continue;
                    }
                    manifest.env_values.push(miedit_core::manifest::EnvCleanupRef {
                        name: entry.name.clone(),
                        machine: entry.machine,
                        path_segment: String::new(),
                        existed: existed.is_some(),
                        previous_value: existed.unwrap_or_default(),
                    });
                    push_log(
                        st,
                        format!("环境变量: {} = {}（{scope_label}，直接设值）", entry.name, entry.value),
                    );
                }
                part @ (miedit_core::envspec::EnvPart::PathAppend
                | miedit_core::envspec::EnvPart::PathPrepend) => {
                    let segment = entry.value.trim().trim_matches('"').to_string();
                    if segment.is_empty() {
                        push_log(st, format!("警告: 环境变量 {} 段为空，已跳过", entry.name));
                        continue;
                    }
                    let current = winutil::env_read(entry.machine, &entry.name)
                        .map(|v| miedit_core::envspec::split_path_list(&v))
                        .unwrap_or_default();
                    let existed = winutil::env_read(entry.machine, &entry.name).is_some();
                    let merged = miedit_core::envspec::merge_path_list(
                        &current,
                        &segment,
                        part == miedit_core::envspec::EnvPart::PathPrepend,
                    );
                    if let Err(e) = winutil::env_write(
                        entry.machine,
                        &entry.name,
                        &miedit_core::envspec::join_path_list(&merged),
                    ) {
                        push_log(st, format!("警告: 环境变量 {} 写入失败: {e:#}", entry.name));
                        continue;
                    }
                    manifest.env_values.push(miedit_core::manifest::EnvCleanupRef {
                        name: entry.name.clone(),
                        machine: entry.machine,
                        path_segment: segment.clone(),
                        existed,
                        previous_value: String::new(),
                    });
                    let part_label = if part == miedit_core::envspec::EnvPart::PathPrepend {
                        "列表前置"
                    } else {
                        "列表追加"
                    };
                    push_log(
                        st,
                        format!("环境变量: {} {} 段 {}（{scope_label}）", entry.name, part_label, segment),
                    );
                }
            }
            env_dirty = true;
        }
        if env_dirty {
            winutil::env_broadcast();
            push_log(st, "环境变更已广播（WM_SETTINGCHANGE），新启动进程立即生效".to_string());
        }
    }
    #[cfg(not(windows))]
    if !cfg.install.env.is_empty() {
        push_log(
            st,
            format!(
                "非 Windows 平台：跳过 {} 条环境变量写入（平台桩，同注册表）",
                cfg.install.env.len()
            ),
        );
    }

    // 5. 卸载清单：Windows 内嵌进卸载器 EXE 尾部（NSIS 式自包含卸载器）；
    //    Linux 落盘 uninstall.json（卸载器自带清单文件回退读取）
    //    卸载器壁纸（步骤 19）随清单/图标一起内嵌；无壁纸时空字节（32B footer bg_len=0）
    let json = serde_json::to_vec_pretty(&manifest)?;
    #[cfg(windows)]
    {
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
    }
    #[cfg(not(windows))]
    {
        let mp = dest.join("uninstall.json");
        fs::write(&mp, &json).with_context(|| format!("写入卸载清单失败: {}", mp.display()))?;
        manifest.manifest_path = mp.display().to_string();
        push_log(st, format!("卸载清单: {}", mp.display()));
    }

    push_log(
        st,
        format!(
            "已写入 {} 个文件、{} 个目录、{} 个快捷方式",
            files.len(),
            dirs.len(),
            manifest.shortcuts.len()
        ),
    );

    // 6. 安装后命令（NSIS ExecWait 语义，步骤 23）：清单/卸载器已就绪，
    //    命令失败（abort_on_error）时安装仍可卸载，安全退出点
    for (i, spec) in cfg.install.exec_after.iter().enumerate() {
        let describe = format!("安装后命令 #{}", i + 1);
        push_log(st, format!("执行{describe}: {}", spec.cmd.trim()));
        let outcome = miedit_core::runcmd::execute(spec, dest);
        for line in miedit_core::runcmd::log_lines(&describe, spec, &outcome) {
            push_log(st, line);
        }
        if miedit_core::runcmd::should_abort(spec, &outcome) {
            anyhow::bail!("{} 失败: {}", describe, miedit_core::runcmd::abort_reason(spec, &outcome));
        }
    }

    Ok(manifest)
}
