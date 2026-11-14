//! MIEdit 安装包编译器（CLI）。
//!
//! 用法：
//! ```text
//! miedit-compiler build <script.miedit.toml> [-o output.exe]
//! ```

use anyhow::{bail, Context, Result};
use miedit_compiler::{build_package, BuildOptions};
use std::fs;
use std::path::PathBuf;

fn main() {
    if let Err(e) = run() {
        eprintln!("错误: {e:#}");
        std::process::exit(1);
    }
}

fn print_usage() {
    println!(
        "MIEdit (Ming-Install-Edit) 编译器 v{}\n\n\
         用法:\n  \
         miedit-compiler build <script.miedit.toml> [-o <output.exe>]\n         \
         [--stub <installer.exe>] [--uninstaller <uninstaller.exe>]\n\n\
         提示: 也可使用 GUI 构建器（miedit-builder）可视化制作安装包。\n\n\
         示例:\n  \
         miedit-compiler build examples/demo-app.miedit.toml -o out/Demo-Setup.exe\n\n\
         默认从可执行文件同目录查找 miedit-installer.exe / miedit-uninstaller.exe（cargo build --release 产物）。",
        miedit_core::VERSION
    );
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" || args[0] == "help" {
        print_usage();
        return Ok(());
    }
    match args[0].as_str() {
        "build" => cmd_build(&args[1..]),
        "--version" | "version" => {
            println!("miedit-compiler {}", miedit_core::VERSION);
            Ok(())
        }
        other => bail!("未知命令: {other}。可用命令: build"),
    }
}

fn cmd_build(args: &[String]) -> Result<()> {
    let mut script: Option<String> = None;
    let mut out: Option<PathBuf> = None;
    let mut stub: Option<PathBuf> = None;
    let mut unp: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--output" => {
                out = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--stub" => {
                stub = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--uninstaller" => {
                unp = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            s => {
                script = Some(s.to_string());
                i += 1;
            }
        }
    }
    let script_path = PathBuf::from(script.ok_or_else(|| {
        anyhow::anyhow!("缺少脚本参数。用法: miedit-compiler build <script.miedit.toml>")
    })?);
    let base = script_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    // 1. 解析脚本
    let text = fs::read_to_string(&script_path)
        .with_context(|| format!("读取脚本失败: {}", script_path.display()))?;
    let cfg: miedit_core::config::Config = toml::from_str(&text)
        .with_context(|| format!("解析脚本失败: {}", script_path.display()))?;

    // 2. 打包
    let mut progress = |m: &str| eprintln!("{m}");
    let summary = build_package(
        &cfg,
        &BuildOptions {
            script_dir: base,
            output: out.unwrap_or_else(|| {
                PathBuf::from(format!(
                    "{}-{}-setup.exe",
                    miedit_core::util::slugify(&cfg.app.name),
                    cfg.app.version
                ))
            }),
            stub,
            uninstaller: unp,
        },
        &mut progress,
    )?;

    // 3. 摘要
    println!("✔ 生成安装包: {}", summary.output.display());
    println!(
        "  总大小 {} | payload {} | 配置 {} | 卸载器 {}",
        miedit_core::util::human_size(summary.total_size),
        miedit_core::util::human_size(summary.payload_len),
        miedit_core::util::human_size(summary.config_len),
        miedit_core::util::human_size(summary.uninstaller_len),
    );
    println!(
        "  应用: {} {} (app_id={}, per_machine={})",
        summary.app_name, summary.version, summary.app_id, summary.per_machine
    );
    Ok(())
}
