//! MIEdit 自解压安装器 Stub。
//!
//! 启动时读取自身 EXE 尾部的包数据：
//! 1. 若脚本声明每机器安装而当前非管理员 → 通过 UAC 提权重启自身
//!    （Linux：per_machine 需 root，未提权时报错退出）；
//! 2. `--silent` 模式直接安装到默认目录（或 `--dir=` 指定目录）并写日志；
//! 3. `--repair` 模式进入修复确认页（ARP ModifyPath 入口，路线图步骤 17）；
//! 4. 否则进入现代化 GUI 安装向导。
//!
//! 完整命令行见 `miedit_core::cliargs::help_text`（路线图步骤 21，
//! Linux/CLI 适配）；退出码 0/1/2 = 成功/失败/参数无效。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod finalize;
mod install;
mod pkg;
mod theme;

use anyhow::{anyhow, Result};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut silent = false;
    let mut dir: Option<String> = None;
    // NSIS CRCCheck 语义：`--ncrc` / `/ncrc` 跳过包完整性 CRC32 校验
    let mut ncrc = false;
    // ARP 修复入口（路线图步骤 17）：`--repair` / `/repair` 进入修复模式
    let mut repair = false;
    // 步骤 21（CLI 适配）：--components= / --no-shortcuts / --help / --version
    let mut components_arg: Option<String> = None;
    let mut no_shortcuts = false;
    for a in &args {
        let low = a.to_ascii_lowercase();
        if low == "--silent" || low == "/s" || low == "-s" {
            silent = true;
        } else if low == "--ncrc" || low == "/ncrc" {
            ncrc = true;
        } else if low == miedit_core::repair::REPAIR_ARG || low == "/repair" {
            repair = true;
        } else if low == "--help" || low == "-h" || low == "/?" || low == "/help" {
            println!(
                "{}",
                miedit_core::cliargs::help_text(
                    "miedit-installer",
                    "MIEdit Installer",
                    miedit_core::VERSION
                )
            );
            std::process::exit(miedit_core::cliargs::EXIT_OK);
        } else if low == "--version" || low == "-v" || low == "-V" {
            println!("MIEdit Installer {}", miedit_core::VERSION);
            std::process::exit(miedit_core::cliargs::EXIT_OK);
        } else if low == "--no-shortcuts" || low == "/no-shortcuts" {
            no_shortcuts = true;
        } else if let Some(rest) = a.strip_prefix("--components=") {
            components_arg = Some(rest.to_string());
        } else if let Some(rest) = a.strip_prefix("--dir=") {
            dir = Some(rest.trim_matches('"').to_string());
        } else if low.starts_with("/d=") {
            dir = Some(a[3..].trim_matches('"').to_string());
        }
    }

    match real_main(silent, ncrc, repair, dir, components_arg, no_shortcuts, &args) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            miedit_core::winutil::message_box_error("MIEdit 安装程序", &format!("{e:#}"));
            std::process::exit(miedit_core::cliargs::EXIT_FAILED);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn real_main(
    silent: bool,
    ncrc: bool,
    repair: bool,
    dir_arg: Option<String>,
    components_arg: Option<String>,
    no_shortcuts: bool,
    args: &[String],
) -> Result<i32> {
    let pkg = if silent {
        // 静默模式：同步校验 CRC（确定性退出码，损坏包拒绝安装）
        pkg::load_self_package(ncrc)?
    } else {
        // GUI 模式（步骤 30 打开速度优化）：只解析配置与图标，CRC 校验与
        // 壁纸解码移入后台线程（提取前由 CrcGate 保证校验完成）
        pkg::load_self_package_fast()?
    };
    let cfg = &pkg.config;

    if cfg.app.name.is_empty() {
        return Err(anyhow!("安装包配置缺少应用名称"));
    }

    // 步骤 34：单实例互斥（NSIS CreateMutex 惯例 / Inno AppMutex 对齐）：
    // 已有另一安装实例运行时提示并退出（GUI 弹消息框，静默写日志）。
    // 守卫 `_instance` 持有到 real_main 结束（进程退出自动释放，双保险）
    let _instance = match miedit_core::winutil::acquire_single_instance(&cfg.app_id_or_slug(), false)
    {
        Some(g) => g,
        None => {
            let msg = "另一个安装程序实例正在运行，请等待其完成后再试。\nAnother installer instance is already running.";
            if silent {
                write_silent_log(&format!("× {msg}"));
            } else {
                miedit_core::winutil::message_box_error("MIEdit Installer", msg);
            }
            return Ok(miedit_core::cliargs::EXIT_FAILED);
        }
    };

    // --components= 解析（Inno /COMPONENTS 语义）：按组件名/序号过滤；
    // 引用不存在的组件 → 退出码 2（参数无效）
    let cli_components = match &components_arg {
        Some(spec) => {
            let names: Vec<String> = cfg
                .components
                .iter()
                .map(|c| {
                    if c.name.trim().is_empty() {
                        "component".to_string()
                    } else {
                        c.name.trim().to_string()
                    }
                })
                .collect();
            match miedit_core::cliargs::parse_components_arg(spec, &names) {
                Ok(sel) => sel,
                Err(()) => {
                    let msg = format!(
                        "--components 参数无效：\"{spec}\"（可用组件: {}）",
                        names.join(", ")
                    );
                    eprintln!("{msg}");
                    write_silent_log(&msg);
                    return Ok(miedit_core::cliargs::EXIT_ARGS);
                }
            }
        }
        None => None,
    };

    // 最小系统版本门槛（Inno MinVersion / NSIS 对齐，路线图步骤 12）：
    // 用 RtlGetVersion 取真实版本，低于门槛直接退出（GUI 弹错误框，静默写日志返回 1）
    let minv = cfg.install.min_version.trim();
    if !minv.is_empty() {
        match miedit_core::sysreq::version_at_least(miedit_core::winutil::windows_version(), minv) {
            Ok(true) => {}
            Ok(false) => {
                let msg = format!(
                    "此程序需要 Windows {} 或更高版本，当前系统版本无法安装。",
                    minv
                );
                if silent {
                    eprintln!("{msg}");
                    write_silent_log(&msg);
                    return Ok(miedit_core::cliargs::EXIT_FAILED);
                }
                return Err(anyhow!("{msg}"));
            }
            // 门槛格式无效：编译期已校验，运行期宽容忽略（不阻断安装）
            Err(_) => {}
        }
    }

    // 每机器安装需要管理员权限：通过 UAC 重启自身（Linux：要求 root 运行）
    if cfg.app.per_machine && !miedit_core::winutil::is_admin() {
        if miedit_core::winutil::relaunch_elevated(args)? {
            return Ok(miedit_core::cliargs::EXIT_OK);
        }
        #[cfg(not(windows))]
        let err = anyhow!(
            "此安装包声明为每机器安装（per_machine），需要以 root 运行（例如 sudo {}）",
            std::env::args().next().unwrap_or_default()
        );
        #[cfg(windows)]
        let err = anyhow!("此安装程序需要管理员权限（每机器安装）");
        return Err(err);
    }

    // 步骤 35：/LANG= 命令行语言覆盖（NSIS /LANG 语义）：优先于系统自动
    // 预选；无效值（未匹配语言包）宽容回退自动预选，仅静默日志记录
    let lang_arg = miedit_core::cliargs::parse_lang_arg(args);
    let lang_override = lang_arg
        .as_deref()
        .and_then(|v| miedit_core::langs::resolve_lang_override(&cfg.ui.languages, v));
    if let (Some(v), None) = (&lang_arg, lang_override) {
        if silent {
            write_silent_log(&format!(
                "提示: /LANG={v} 未匹配到语言包，回退自动预选"
            ));
        }
    }

    let dest = match dir_arg {
        Some(d) if !d.trim().is_empty() => std::path::PathBuf::from(d.trim()),
        _ => {
            // NSIS InstallDirRegKey 行为：优先沿用上次安装目录
            miedit_core::winutil::read_prev_install_dir(cfg.app.per_machine, &cfg.app_id_or_slug())
                .unwrap_or_else(|| {
                    miedit_core::util::resolve_path_template(&cfg.install.dir_template, &cfg.app.name)
                })
        }
    };

    if silent {
        // 静默模式无语言选择页：/LANG= 覆盖优先（步骤 35），否则按系统语言
        // 自动预选（NSIS /S 语义）。静默修复（--silent --repair）与静默安装
        // 行为一致：全量重装即静默修复。步骤 21：--components= 过滤组件；
        // --no-shortcuts 跳过快捷方式。
        let language = lang_override.or_else(|| miedit_core::langs::initial_language(cfg));
        let rules = cli_components.as_ref().map(|sel| {
            miedit_core::installtypes::selection_to_rules(
                &pkg.config.components,
                pkg.config.files.len(),
                sel,
            )
        });
        let st = install::spawn(
            pkg.clone(),
            dest,
            None,
            rules,
            cli_components.clone(),
            language,
            no_shortcuts,
            None,
        );
        // 等待安装线程结束（join handle 未暴露，轮询即可）
        loop {
            {
                let s = st.lock().unwrap();
                match &s.phase {
                    install::Phase::Done => break,
                    install::Phase::Failed(_) => {
                        let log = s.log.join("\n");
                        write_silent_log(&log);
                        return Ok(miedit_core::cliargs::EXIT_FAILED);
                    }
                    _ => {}
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let log = st.lock().unwrap().log.join("\n");
        write_silent_log(&log);
        Ok(miedit_core::cliargs::EXIT_OK)
    } else {
        app::run_gui(pkg, dest, repair, cli_components, no_shortcuts, ncrc, lang_override)?;
        Ok(miedit_core::cliargs::EXIT_OK)
    }
}

fn write_silent_log(log: &str) {
    // Linux：无 TEMP 时回退 /tmp（步骤 21 CLI 适配）
    let tmp = std::env::var("TEMP")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| std::env::var("TMPDIR").ok().filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| "/tmp".to_string());
    let p = std::path::PathBuf::from(tmp).join("miedit-install.log");
    let _ = std::fs::write(&p, log);
}
