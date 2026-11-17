//! 后台安装线程：解包 payload → 写盘 → 调用 finalize 收尾。

use crate::finalize;
use crate::pkg::Package;
use anyhow::{Context as _, Result};
use eframe::egui;
use miedit_core::archive::ArchiveReader;
use miedit_core::fileerror::{
    self, FileErrorChoice, PromptChannel, WriteFailurePlan,
};
use miedit_core::overwrite::{decide, OverwriteDecision, OverwriteMode};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Phase {
    #[default]
    Idle,
    Extracting,
    Finalizing,
    Done,
    Failed(String),
}

#[derive(Debug, Default)]
pub struct ProgressState {
    pub phase: Phase,
    pub current_file: String,
    pub done_bytes: u64,
    pub total_bytes: u64,
    /// 文件计数（归档头 v2 / 组件过滤后）
    pub total_files: u64,
    pub done_files: u64,
    /// 开始时间（计算速度）
    pub started: Option<std::time::Instant>,
    pub log: Vec<String>,
    /// 步骤 8：文件写入错误的「请求-应答」信箱（后台线程 ↔ UI 模态卡片）
    pub prompt_channel: PromptChannel,
    /// 步骤 9：已计划重启替换的文件（包内相对路径）
    pub pending_reboot: Vec<String>,
}

/// 向安装日志追加一行（保留最近 500 条）。
pub fn push_log(st: &Mutex<ProgressState>, line: impl Into<String>) {
    let mut s = st.lock().unwrap();
    s.log.push(line.into());
    let len = s.log.len();
    if len > 500 {
        s.log.drain(0..len - 500);
    }
}

/// 打开文件错误模态并阻塞等待用户答复（NSIS 文件错误对话框为模态：
/// 安装暂停直至用户选择重试/忽略/中止）。UI 线程每帧轮询
/// `ProgressState::prompt_channel` 渲染卡片并写入答复。
fn wait_file_error_choice(
    st: &Mutex<ProgressState>,
    ctx: &egui::Context,
    rel: &str,
    err: &anyhow::Error,
) -> FileErrorChoice {
    {
        let mut s = st.lock().unwrap();
        s.prompt_channel.open_request(rel, format!("{err:#}"));
    }
    loop {
        ctx.request_repaint();
        std::thread::sleep(std::time::Duration::from_millis(60));
        let mut s = st.lock().unwrap();
        if let Some(choice) = s.prompt_channel.take_reply() {
            return choice;
        }
    }
}

/// 启动后台安装线程，返回共享进度状态。
/// `selected_rules`：组件过滤后允许安装的 `[[files]]` 规则索引；None = 全部安装。
/// `language`：选中语言索引（步骤 15 多语言；None = 无语言包，文案用英文默认）。
pub fn spawn(
    pkg: Arc<Package>,
    dest: PathBuf,
    ctx: Option<egui::Context>,
    selected_rules: Option<std::collections::HashSet<usize>>,
    language: Option<usize>,
) -> Arc<Mutex<ProgressState>> {
    let st = Arc::new(Mutex::new(ProgressState {
        phase: Phase::Extracting,
        total_bytes: pkg.total_bytes,
        started: Some(std::time::Instant::now()),
        ..Default::default()
    }));
    push_log(&st, format!("目标目录: {}", dest.display()));
    let st2 = st.clone();
    std::thread::Builder::new()
        .name("mris-install".into())
        .spawn(move || {
            let res = run_install(&pkg, &dest, &st2, ctx.as_ref(), selected_rules.as_ref(), language);
            let mut s = st2.lock().unwrap();
            match res {
                Ok(()) => {
                    s.phase = Phase::Done;
                    s.log.push("√ 安装完成".into());
                }
                Err(e) => {
                    s.phase = Phase::Failed(format!("{e:#}"));
                    s.log.push(format!("× 安装失败: {e:#}"));
                }
            }
            drop(s);
            if let Some(c) = ctx {
                c.request_repaint();
            }
        })
        .expect("启动安装线程失败");
    st
}

fn record_dir(
    dirs: &mut Vec<String>,
    seen: &mut HashSet<String>,
    rel: &str,
) {
    // 逐级记录父目录，保证卸载时可自底向上删除
    let parts: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    let mut acc = String::new();
    for p in parts {
        if !acc.is_empty() {
            acc.push('/');
        }
        acc.push_str(p);
        if seen.insert(acc.clone()) {
            dirs.push(acc.clone());
        }
    }
}

fn run_install(
    pkg: &Package,
    dest: &Path,
    st: &Mutex<ProgressState>,
    ctx: Option<&egui::Context>,
    selected_rules: Option<&std::collections::HashSet<usize>>,
    language: Option<usize>,
) -> Result<()> {
    // 1. 旧版本检测（升级提示 + 静默卸载旧版）：新版为内嵌清单，旧版兼容 uninstall.json
    let old_exe = dest.join("uninstall.exe");
    let old_json = dest.join("uninstall.json");
    if old_exe.is_file() {
        let old_version: Option<String> =
            miedit_core::trailer::read_uninstall_data(&old_exe)
                .and_then(|(m, _, _)| serde_json::from_slice::<serde_json::Value>(&m).ok())
                .and_then(|v| v.get("version").and_then(|x| x.as_str()).map(String::from))
                .or_else(|| {
                    std::fs::read(&old_json)
                        .ok()
                        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                        .and_then(|v| v.get("version").and_then(|x| x.as_str()).map(String::from))
                });
        if let Some(old) = &old_version {
            if old != &pkg.config.app.version {
                push_log(
                    st,
                    format!(
                        "检测到已安装版本 {}，将升级到 {}",
                        old, pkg.config.app.version
                    ),
                );
            }
        }
        push_log(st, "检测到已安装的旧版本，正在静默卸载...");
        match std::process::Command::new(&old_exe).arg("--silent").status() {
            Ok(s) if s.success() => push_log(st, "旧版本已卸载"),
            Ok(s) => push_log(st, format!("旧版本卸载器退出码非 0: {s}")),
            Err(e) => push_log(st, format!("旧版本卸载失败: {e}")),
        }
    }

    // 2. 主程序运行中检测（静默模式同样提示并跳过）
    if !pkg.config.app.exe.is_empty() {
        let exe = dest.join(&pkg.config.app.exe);
        if miedit_core::winutil::is_file_locked(&exe) {
            push_log(st, "警告: 主程序似乎正在运行，可能无法覆盖更新");
        }
    }

    fs::create_dir_all(dest).with_context(|| format!("创建目录失败: {}", dest.display()))?;

    // 3. 组件过滤：把选中规则展开为允许安装的包内路径集合
    let allowed: Option<(HashSet<String>, u64)> = selected_rules.map(|rules| {
        let mut set = HashSet::new();
        let mut total = 0u64;
        for &r in rules {
            if let Some(paths) = pkg.config.files_map.get(r) {
                for fr in paths {
                    set.insert(fr.path.clone());
                    total += fr.size;
                }
            }
        }
        (set, total)
    });
    if let Some((a, total)) = &allowed {
        push_log(st, format!("组件过滤: 已选 {} 个文件, {}", a.len(), miedit_core::util::human_size(*total)));
        let mut s0 = st.lock().unwrap();
        s0.total_bytes = *total;
        s0.total_files = a.len() as u64;
    }

    // 4. 解包（NSIS 风格 solid 整包压缩 或 条目级 deflate）
    let raw = miedit_core::trailer::read_range(&mut fs::File::open(&pkg.path)?, pkg.trailer.payload)?;
    let archive: Vec<u8> = if pkg.config.install.compression == "solid" {
        use flate2::read::DeflateDecoder;
        use std::io::Read as _;
        push_log(st, "解压 Deflate solid 归档（整包）...");
        let mut out = Vec::new();
        DeflateDecoder::new(&raw[..]).read_to_end(&mut out).context("solid 解压失败")?;
        out
    } else {
        raw
    };
    let mut reader = ArchiveReader::new(&archive[..]).context("安装包 payload 无效")?;
    {
        let mut s0 = st.lock().unwrap();
        if s0.total_files == 0 {
            s0.total_files = reader.file_count();
        }
    }

    let mut files: Vec<String> = Vec::new();
    let mut dirs: Vec<String> = Vec::new();
    let mut seen_dirs = HashSet::new();

    // NSIS SetOverwrite：包内源文件修改时间（FileRef::mtime，编译器写入）
    let mode = OverwriteMode::parse(&pkg.config.install.overwrite);
    push_log(
        st,
        match mode {
            OverwriteMode::On => "覆盖策略: overwrite = on（总是覆盖）".to_string(),
            OverwriteMode::Off => "覆盖策略: overwrite = off（已存在则跳过）".to_string(),
            OverwriteMode::Try => "覆盖策略: overwrite = try（失败忽略）".to_string(),
            OverwriteMode::IfNewer => "覆盖策略: overwrite = ifnewer（仅源更新时覆盖）".to_string(),
        },
    );
    let mut mtime_map: HashMap<String, u64> = HashMap::new();
    for rule in &pkg.config.files_map {
        for fr in rule {
            mtime_map.entry(fr.path.clone()).or_insert(fr.mtime);
        }
    }

    while let Some(entry) = reader.next_entry().context("读取归档条目失败")? {
        if !miedit_core::files::is_safe_arc_path(&entry.path) {
            anyhow::bail!("归档包含不安全路径: {}", entry.path);
        }
        let rel = entry.path.replace('\\', "/");
        if let Some((allow, _)) = &allowed {
            if !entry.is_dir && !allow.contains(&rel) {
                continue; // 未选中的组件文件
            }
        }
        let target = dest.join(&rel);

        if entry.is_dir {
            fs::create_dir_all(&target).with_context(|| format!("创建目录失败: {rel}"))?;
            record_dir(&mut dirs, &mut seen_dirs, &rel);
            continue;
        }

        let data = entry
            .decode()
            .with_context(|| format!("解压失败: {}", entry.path))?;
        if let Some(parent) = target.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }
        if let Some(pr) = target.parent() {
            if let Ok(prel) = pr.strip_prefix(dest) {
                let prel = prel.to_string_lossy().replace('\\', "/");
                if !prel.is_empty() {
                    record_dir(&mut dirs, &mut seen_dirs, &prel);
                }
            }
        }

        // NSIS SetOverwrite：按覆盖策略决定写入行为
        let target_mtime = target
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let new_mtime = mtime_map.get(&rel).copied().unwrap_or(0);
        let decision = decide(mode, target.exists(), target_mtime, new_mtime);
        if decision == OverwriteDecision::Skip {
            push_log(st, format!("已存在，跳过: {rel}（overwrite = off/ifnewer）"));
            continue;
        }

        // 写入；失败时进入 NSIS 文件错误处置（步骤 8 重试/忽略/中止 + 步骤 9 重启替换）
        let do_write = || -> Result<()> {
            let mut out = BufWriter::new(fs::File::create(&target).with_context(|| {
                format!("创建文件失败: {}", target.display())
            })?);
            out.write_all(&data)?;
            out.flush()?;
            Ok(())
        };
        if let Err(mut err) = do_write() {
            loop {
                // 按覆盖策略与运行模式决定处置计划
                let plan = if !fileerror::needs_prompt(mode) {
                    // overwrite = try：NSIS SetOverwrite try 语义，失败不中断；同时计划重启替换
                    push_log(
                        st,
                        format!("警告: 覆盖失败已忽略: {rel}（overwrite = try）: {err:#}"),
                    );
                    fileerror::try_mode_plan()
                } else if let Some(c) = ctx {
                    // 交互模式：弹出「重试 / 忽略 / 中止」模态，阻塞等待用户选择
                    match wait_file_error_choice(st, c, &rel, &err) {
                        FileErrorChoice::Retry => WriteFailurePlan::RetryWrite,
                        FileErrorChoice::Ignore => {
                            push_log(st, format!("警告: 覆盖失败已忽略（用户选择忽略）: {rel}: {err:#}"));
                            WriteFailurePlan::Skip { reboot_replace: true }
                        }
                        FileErrorChoice::Abort => WriteFailurePlan::Abort,
                    }
                } else {
                    // 静默模式无 GUI：默认忽略 + 计划重启替换（设计取舍见 lessons 文档）
                    push_log(
                        st,
                        format!("警告: 写入失败已忽略（静默模式，已计划重启替换）: {rel}: {err:#}"),
                    );
                    fileerror::silent_default_plan()
                };
                match plan {
                    WriteFailurePlan::RetryWrite => {
                        push_log(st, format!("重试写入: {rel}"));
                        match do_write() {
                            Ok(()) => break, // 重试成功，继续后续流程（时间戳/计数）
                            Err(e) => err = e, // 仍失败：再次征求处置
                        }
                    }
                    WriteFailurePlan::Skip { reboot_replace } => {
                        if reboot_replace {
                            // 步骤 9：计划重启替换（NSIS SetRebootFlag / Inno restartreplace）
                            match miedit_core::winutil::schedule_replace_on_reboot(&target, &data) {
                                Ok(immediate) => {
                                    push_log(
                                        st,
                                        format!(
                                            "已计划重启替换: {rel}{}",
                                            if immediate {
                                                "（旧文件已腾位，新内容已写入）"
                                            } else {
                                                "（重启后原子替换）"
                                            },
                                        ),
                                    );
                                    {
                                        let mut s = st.lock().unwrap();
                                        s.pending_reboot.push(rel.clone());
                                    }
                                    // 目标文件（重启后）会存在，登记进卸载清单保证可清理
                                    files.push(target.display().to_string());
                                }
                                Err(e2) => {
                                    push_log(st, format!("警告: 计划重启替换失败: {rel}: {e2:#}"));
                                }
                            }
                        }
                        break; // 跳过该文件继续安装
                    }
                    WriteFailurePlan::Abort => {
                        return Err(anyhow::anyhow!(fileerror::abort_message(&rel, files.len())));
                    }
                }
            }
        }
        // 保留源文件修改时间（NSIS 做法；保证 ifnewer 升级比较正确）
        if new_mtime > 0 {
            if let Err(e) = miedit_core::winutil::set_file_mtime(&target, new_mtime) {
                push_log(st, format!("提示: 设置文件时间失败: {rel}: {e:#}"));
            }
        }
        files.push(target.display().to_string());

        {
            let mut s = st.lock().unwrap();
            s.current_file = rel.clone();
            s.done_bytes += entry.orig_len;
            s.done_files += 1;
        }
        if let Some(c) = ctx {
            c.request_repaint();
        }
    }

    // 3. 收尾：卸载器、快捷方式、注册表、清单
    {
        let mut s = st.lock().unwrap();
        s.phase = Phase::Finalizing;
        s.current_file.clear();
        s.done_bytes = s.total_bytes;
    }
    if let Some(c) = ctx {
        c.request_repaint();
    }
    finalize::finalize_install(pkg, dest, &files, &dirs, st, language)?;

    // NSIS LogSet 风格：安装日志落盘到 $INSTDIR\install.log（可用 install.write_log 关闭）
    if pkg.config.install.write_log {
        let log_text = {
            let s = st.lock().unwrap();
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let mut t = format!(
                "MIEdit 安装日志\r\n应用: {} {}\r\n目标目录: {}\r\n时间戳: {} (Unix)\r\n\r\n",
                pkg.config.app.name,
                pkg.config.app.version,
                dest.display(),
                ts
            );
            for line in &s.log {
                t.push_str(line);
                t.push_str("\r\n");
            }
            t
        };
        let log_path = dest.join("install.log");
        match fs::write(&log_path, log_text) {
            Ok(()) => push_log(st, format!("已写入安装日志: {}", log_path.display())),
            Err(e) => push_log(st, format!("警告: 写入安装日志失败: {e}")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    // 组件过滤契约：选中规则 → 展开为允许安装的包内路径集合
    #[test]
    fn test_component_filter_mapping() {
        use miedit_core::config::FileRef;
        let files_map = vec![
            vec![FileRef { path: "a.exe".into(), size: 1, mtime: 0 }],
            vec![FileRef { path: "docs/x.md".into(), size: 1, mtime: 0 }],
            vec![FileRef { path: "data/c.json".into(), size: 1, mtime: 0 }],
        ];
        let rules = std::collections::HashSet::from([0usize, 2]);
        let mut allowed = std::collections::HashSet::new();
        for &r in &rules {
            if let Some(paths) = files_map.get(r) {
                for fr in paths {
                    allowed.insert(fr.path.clone());
                }
            }
        }
        assert!(allowed.contains("a.exe"));
        assert!(allowed.contains("data/c.json"));
        assert!(!allowed.contains("docs/x.md"));

        // 未选中任何规则 → 空集合（不安装组件文件）
        let empty = std::collections::HashSet::<usize>::new();
        let mut none = std::collections::HashSet::new();
        for r in &empty {
            if let Some(paths) = files_map.get(*r) {
                for fr in paths {
                    none.insert(fr.path.clone());
                }
            }
        }
        assert!(none.is_empty());
    }
}
