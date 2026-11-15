//! MIEdit 自解压安装器 Stub。
//!
//! 启动时读取自身 EXE 尾部的包数据：
//! 1. 若脚本声明每机器安装而当前非管理员 → 通过 UAC 提权重启自身；
//! 2. `--silent` 模式直接安装到默认目录（或 `--dir=` 指定目录）并写日志；
//! 3. 否则进入现代化 GUI 安装向导。

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
    for a in &args {
        let low = a.to_ascii_lowercase();
        if low == "--silent" || low == "/s" || low == "-s" {
            silent = true;
        } else if low == "--ncrc" || low == "/ncrc" {
            ncrc = true;
        } else if let Some(rest) = a.strip_prefix("--dir=") {
            dir = Some(rest.trim_matches('"').to_string());
        } else if low.starts_with("/d=") {
            dir = Some(a[3..].trim_matches('"').to_string());
        }
    }

    match real_main(silent, ncrc, dir, &args) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            miedit_core::winutil::message_box_error("MIEdit 安装程序", &format!("{e:#}"));
            std::process::exit(1);
        }
    }
}

fn real_main(silent: bool, ncrc: bool, dir_arg: Option<String>, args: &[String]) -> Result<i32> {
    let pkg = pkg::load_self_package(ncrc)?;
    let cfg = &pkg.config;

    if cfg.app.name.is_empty() {
        return Err(anyhow!("安装包配置缺少应用名称"));
    }

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
                    return Ok(1);
                }
                return Err(anyhow!("{msg}"));
            }
            // 门槛格式无效：编译期已校验，运行期宽容忽略（不阻断安装）
            Err(_) => {}
        }
    }

    // 每机器安装需要管理员权限：通过 UAC 重启自身
    if cfg.app.per_machine && !miedit_core::winutil::is_admin() {
        if miedit_core::winutil::relaunch_elevated(args)? {
            return Ok(0);
        }
        return Err(anyhow!("此安装程序需要管理员权限（每机器安装）"));
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
        // 静默模式无语言选择页：按系统语言自动预选（NSIS /S 语义）
        let language = miedit_core::langs::initial_language(cfg);
        let st = install::spawn(pkg.clone(), dest, None, None, language);
        // 等待安装线程结束（join handle 未暴露，轮询即可）
        loop {
            {
                let s = st.lock().unwrap();
                match &s.phase {
                    install::Phase::Done => break,
                    install::Phase::Failed(_) => {
                        let log = s.log.join("\n");
                        write_silent_log(&log);
                        return Ok(1);
                    }
                    _ => {}
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let log = st.lock().unwrap().log.join("\n");
        write_silent_log(&log);
        Ok(0)
    } else {
        app::run_gui(pkg, dest)?;
        Ok(0)
    }
}

fn write_silent_log(log: &str) {
    if let Ok(tmp) = std::env::var("TEMP") {
        let p = std::path::PathBuf::from(tmp).join("miedit-install.log");
        let _ = std::fs::write(&p, log);
    }
}
