//! MIEdit 安装包编译器（CLI，命令头 `mis`）。
//!
//! 用法：
//! ```text
//! mis build <script.mis> [-o output.exe]
//! mis --version
//! ```
//!
//! 输出目录规则（用户指令 2026-11-19）：
//! 1. `-o` 显式指定优先；
//! 2. 否则用脚本 `[build] output_dir`（相对路径相对当前工作目录）；
//! 3. 都未写 → 默认 `<命令行所在目录>/output/<slug>-<version>-setup.exe`
//!    （目录自动创建）。

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
         mis build <script.mis> [-o <output.exe>]\n         \
         [--stub <installer.exe>] [--uninstaller <uninstaller.exe>]\n\n\
         输出目录:\n  \
         1) -o 显式指定优先；\n  \
         2) 脚本 [build] output_dir（相对路径相对当前工作目录）；\n  \
         3) 都未写 → <命令行所在目录>/output/<应用名>-<版本>-setup.exe（自动创建目录）。\n\n\
         提示: 也可使用 GUI 构建器（miedit-builder）可视化制作安装包；\n  \
         在其「设置」菜单可把 mis 所在目录写入 Path 环境变量（默认当前用户）。\n\n\
         示例:\n  \
         mis build examples/demo-app.mis\n\n\
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
        "--version" | "-V" | "version" => {
            println!("mis {}", miedit_core::VERSION);
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
    let script_arg = script.ok_or_else(|| anyhow::anyhow!("缺少脚本参数。用法: mis build <script.mis>"))?;
    let script_path = PathBuf::from(&script_arg);
    let base = script_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    // 1. 解析脚本
    let text = fs::read_to_string(&script_path)
        .with_context(|| format!("读取脚本失败: {}（扩展名 .mis / .mi 均可）", script_path.display()))?;
    let cfg: miedit_core::config::Config = toml::from_str(&text)
        .with_context(|| format!("解析脚本失败: {}（可用 `mis build` 前先在 GUI「MI 脚本」页做语法检测）", script_path.display()))?;

    // 2. 输出路径三层规则（-o > [build] output_dir > <cwd>/output/）
    let slug = miedit_core::util::slugify(&cfg.app.name);
    let version = if cfg.app.version.trim().is_empty() { "1.0.0" } else { cfg.app.version.trim() };
    let file_name = format!("{slug}-{version}-setup.exe");
    let out_explicit = out.is_some();
    let (output, default_dir_used) = match out {
        Some(o) => (o, false),
        None => match cfg.output_dir_opt() {
            Some(dir) => (PathBuf::from(dir).join(&file_name), false),
            None => {
                let cwd = std::env::current_dir().context("获取当前工作目录失败")?;
                (cwd.join("output").join(&file_name), true)
            }
        },
    };
    // 默认目录 / 脚本 output_dir 都自动创建（显式 -o 保持原行为：缺目录由编译器报错）
    if !out_explicit {
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("创建输出目录失败: {}", parent.display()))?;
        }
    }

    // 3. 打包
    let mut progress = |m: &str| eprintln!("{m}");
    let summary = build_package(
        &cfg,
        &BuildOptions {
            script_dir: base,
            output,
            stub,
            uninstaller: unp,
        },
        &mut progress,
    )?;

    // 4. 摘要
    println!("✔ 生成安装包: {}", summary.output.display());
    if default_dir_used {
        println!("  （脚本未写 [build] output_dir，已默认输出到 <当前目录>/output/）");
    }
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
