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
use std::sync::{Arc, Condvar, Mutex};

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

/// CRC 后台校验门（步骤 30 安装包打开速度优化）：GUI 模式窗口先开，
/// CRC 在后台线程验证；安装线程在**提取任何文件之前**等待校验结果——
/// 损坏包仍然拒绝安装（NSIS CRCCheck 语义不变），只是不再阻塞窗口打开。
/// None（已完成）状态用于静默模式：同步校验后直接放行。
pub struct CrcGate {
    result: Mutex<Option<Result<(), String>>>,
    cv: Condvar,
}

impl CrcGate {
    pub fn new_pending() -> Self {
        Self {
            result: Mutex::new(None),
            cv: Condvar::new(),
        }
    }

    /// 后台校验线程写结果并唤醒等待者。
    pub fn complete(&self, res: Result<(), String>) {
        let mut g = self.result.lock().unwrap();
        *g = Some(res);
        self.cv.notify_all();
    }

    /// 安装线程在提取前调用：阻塞至校验完成，失败则 bail（不写任何文件）。
    pub fn wait_ok(&self, ctx: Option<&egui::Context>) -> Result<()> {
        let mut g = self.result.lock().unwrap();
        while g.is_none() {
            if let Some(c) = ctx {
                c.request_repaint();
            }
            g = self.cv.wait(g).unwrap();
        }
        match g.as_ref().unwrap() {
            Ok(()) => Ok(()),
            Err(msg) => Err(anyhow::anyhow!("{msg}")),
        }
    }

    /// 已完成状态（静默模式 / --ncrc）：直接返回，不等待。
    pub fn ready(verified: Result<(), String>) -> Arc<Self> {
        let g = Self::new_pending();
        g.complete(verified);
        Arc::new(g)
    }

    /// UI 轮询：后台校验已失败则返回错误文案（立即转错误页，
    /// 不等到用户点安装才发现）。
    pub fn failed_msg(&self) -> Option<String> {
        match &*self.result.lock().unwrap() {
            Some(Err(msg)) => Some(msg.clone()),
            _ => None,
        }
    }
}

/// 启动后台安装线程，返回共享进度状态。
/// `selected_rules`：组件过滤后允许安装的 `[[files]]` 规则索引；None = 全部安装。
/// `comp_sel`：组件勾选布尔表（MI 脚本 v2 指令层，步骤 33：NSIS Section
/// 语义——required 或勾选的组件才执行其 `code` 指令）；None = 未启用组件过滤。
/// `language`：选中语言索引（步骤 15 多语言；None = 无语言包，文案用英文默认）。
/// `shortcuts_enabled`：是否创建快捷方式（步骤 21：CLI `--no-shortcuts`；GUI 默认 true）。
/// `crc_gate`：后台 CRC 校验门（步骤 30；None = 已验证）。
pub fn spawn(
    pkg: Arc<Package>,
    dest: PathBuf,
    ctx: Option<egui::Context>,
    selected_rules: Option<std::collections::HashSet<usize>>,
    comp_sel: Option<Vec<bool>>,
    language: Option<usize>,
    shortcuts_enabled: bool,
    crc_gate: Option<Arc<CrcGate>>,
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
            // 步骤 30：提取前等待后台 CRC 校验完成（窗口已先开，
            // 若用户比校验更快点到了安装页，这里短暂等待；损坏包拒绝安装）
            if let Some(gate) = &crc_gate {
                push_log(&st2, "正在校验安装包完整性…");
                if let Err(e) = gate.wait_ok(ctx.as_ref()) {
                    let mut s = st2.lock().unwrap();
                    s.phase = Phase::Failed(format!("{e:#}"));
                    s.log.push(format!("× 安装失败: {e:#}"));
                    drop(s);
                    if let Some(c) = ctx {
                        c.request_repaint();
                    }
                    return;
                }
                push_log(&st2, "√ 安装包校验通过");
            }
            let res = run_install(
                &pkg,
                &dest,
                &st2,
                ctx.as_ref(),
                selected_rules.as_ref(),
                comp_sel.as_deref(),
                language,
                shortcuts_enabled,
            );
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
    comp_sel: Option<&[bool]>,
    language: Option<usize>,
    shortcuts_enabled: bool,
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

    // 安装前命令（NSIS .onInit/Exec 语义，路线图步骤 23）：解包前执行，
    // GUI 与静默模式同生效；abort_on_error 时中止（文件尚未写出，无残留）
    for (i, spec) in pkg.config.install.exec_before.iter().enumerate() {
        let describe = format!("安装前命令 #{}", i + 1);
        push_log(st, format!("执行{describe}: {}", spec.cmd.trim()));
        let outcome = miedit_core::runcmd::execute(spec, dest);
        for line in miedit_core::runcmd::log_lines(&describe, spec, &outcome) {
            push_log(st, line);
        }
        if miedit_core::runcmd::should_abort(spec, &outcome) {
            anyhow::bail!(
                "{} 失败: {}",
                describe,
                miedit_core::runcmd::abort_reason(spec, &outcome)
            );
        }
    }

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

    // 4. 解包（NSIS SetCompressor 对齐：solid 整包 Deflate / lzma-solid / brotli-solid，或条目级压缩）
    let raw = miedit_core::trailer::read_range(&mut fs::File::open(&pkg.path)?, pkg.trailer.payload)?;
    use miedit_core::archive::{decompress_bytes, Algo};
    let archive: Vec<u8> = match pkg.config.install.compression.as_str() {
        "solid" => {
            push_log(st, "解压 Deflate solid 归档（整包）...");
            decompress_bytes(Algo::Deflate, &raw).context("solid 解压失败")?
        }
        "lzma-solid" => {
            push_log(st, "解压 LZMA solid 归档（整包）...");
            decompress_bytes(Algo::Lzma, &raw).context("lzma solid 解压失败")?
        }
        "brotli-solid" => {
            push_log(st, "解压 Brotli solid 归档（整包）...");
            decompress_bytes(Algo::Brotli, &raw).context("brotli solid 解压失败")?
        }
        _ => raw, // 条目级压缩（deflate/lzma/brotli）由条目标志位在解码时区分
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

        // 磁盘空间预检（步骤 38，NSIS 磁盘满中止语义）：目标盘剩余空间
        // 覆盖不住「新文件 + 余量」时中止（覆盖写可抵扣旧文件占位）。
        // 剩余空间不可知（disk_free_space = None）时跳过检查不误报。
        if entry.orig_len > 0 {
            if let Some(free) = miedit_core::winutil::disk_free_space(&target) {
                let existing = target.metadata().map(|m| m.len()).unwrap_or(0);
                if miedit_core::files::is_space_insufficient(free, entry.orig_len, existing) {
                    let msg = miedit_core::files::space_error_text(free, entry.orig_len, &rel);
                    push_log(st, format!("错误: {msg}"));
                    anyhow::bail!(msg);
                }
            }
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

    // 2b. MI 脚本 v2 组件指令（步骤 33）：勾选组件的 code 指令按脚本顺序执行
    //     （NSIS Section 指令语义：未勾选组件不执行；副作用登记进卸载清单）
    let mut code_effects = finalize::CodeEffects::default();
    run_component_code(pkg, dest, comp_sel, st, ctx, &mut dirs, &mut seen_dirs, &mut code_effects)?;

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
    finalize::finalize_install(pkg, dest, &files, &dirs, st, language, shortcuts_enabled, &code_effects)?;

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

/// MI 脚本 v2（路线图步骤 33）：执行勾选组件的 `code` 指令。
///
/// NSIS Section 指令语义：组件被勾选（或 required）才执行，按脚本顺序；
/// 未启用组件过滤（`comp_sel` 为 None）时全部组件执行。指令失败按 NSIS
/// error-flag 语义**记警告继续**（不中断安装）；`run` 指令的
/// `abort_on_error` 是显式选择的中止开关（对齐 `[[install.exec_before]]`）。
///
/// 副作用登记（经 [`finalize::CodeEffects`] 进卸载清单，卸载可精确清理）：
/// - `write_reg` → `registry_keys` + `registry_values`（新建键/既有键判定）；
/// - `create_dir` → 安装目录内记 `dirs`（相对路径），目录外记 `extra_dirs`。
fn run_component_code(
    pkg: &Package,
    dest: &Path,
    comp_sel: Option<&[bool]>,
    st: &Mutex<ProgressState>,
    ctx: Option<&egui::Context>,
    dirs: &mut Vec<String>,
    seen_dirs: &mut HashSet<String>,
    effects: &mut finalize::CodeEffects,
) -> Result<()> {
    let cfg = &pkg.config;
    if cfg.components.iter().all(|c| c.code.is_empty()) {
        return Ok(()); // 无指令组件：零开销（旧脚本行为不变）
    }
    let vars = miedit_core::micmd::MiVars {
        install_dir: dest,
        exe_dir: &std::env::current_exe().unwrap_or_else(|_| dest.to_path_buf()),
        per_machine: cfg.app.per_machine,
    }
    .standard_map();
    for (ci, comp) in cfg.components.iter().enumerate() {
        if comp.code.is_empty() {
            continue;
        }
        // NSIS：未勾选 Section 的指令不执行（required 锁定勾选）
        let checked = comp_sel.map_or(true, |v| {
            comp.required || v.get(ci).copied().unwrap_or(true)
        });
        if !checked {
            push_log(st, format!("组件「{}」未勾选，跳过 {} 条指令", comp.name, comp.code.len()));
            continue;
        }
        push_log(st, format!("执行组件「{}」指令（{} 条）…", comp.name, comp.code.len()));
        for (ii, inst) in comp.code.iter().enumerate() {
            // NSIS DetailPrint：纯日志行
            if let miedit_core::micmd::MiInstruction::DetailPrint { text } = inst {
                push_log(st, format!("[{}] {}", comp.name, miedit_core::micmd::expand_vars(text, &vars)));
                continue;
            }
            let desc = miedit_core::micmd::describe(inst, Some(&vars));
            push_log(st, format!("指令 #{}: {}", ii + 1, desc));
            let res = execute_instruction(inst, dest, &vars, effects, dirs, seen_dirs, st, &comp.name);
            // NSIS error-flag 语义：指令失败默认记警告继续；仅 `run.abort_on_error`
            // 是显式中止开关（对齐 `[[install.exec_before]]` 契约）
            let fatal = matches!(inst, miedit_core::micmd::MiInstruction::Run { abort_on_error: true, .. });
            match res {
                Err(e) if fatal => {
                    anyhow::bail!("组件「{}」指令 #{} 失败（abort_on_error）: {e}", comp.name, ii + 1);
                }
                Err(e) => {
                    push_log(st, format!("警告: 组件「{}」指令 #{} 失败（继续安装）: {e}", comp.name, ii + 1));
                }
                Ok(()) => {}
            }
            if let Some(c) = ctx {
                c.request_repaint();
            }
        }
    }
    Ok(())
}

/// 执行单条指令（`DetailPrint` 已在上层处理）；失败返回错误描述——
/// 是否中止由上层按指令类型判定（见 `run_component_code`）。
#[allow(clippy::too_many_arguments)]
fn execute_instruction(
    inst: &miedit_core::micmd::MiInstruction,
    dest: &Path,
    vars: &std::collections::BTreeMap<String, String>,
    effects: &mut finalize::CodeEffects,
    dirs: &mut Vec<String>,
    seen_dirs: &mut HashSet<String>,
    st: &Mutex<ProgressState>,
    comp_name: &str,
) -> std::result::Result<(), String> {
    let ex = |s: &str| miedit_core::micmd::expand_vars(s, vars);
    match inst {
        miedit_core::micmd::MiInstruction::Run { cmd, args, working_dir, abort_on_error } => {
            let spec = miedit_core::config::RunCommandSpec {
                cmd: ex(cmd),
                args: ex(args),
                working_dir: ex(working_dir),
                abort_on_error: *abort_on_error,
                optional: false,
            };
            let outcome = miedit_core::runcmd::execute(&spec, dest);
            for line in miedit_core::runcmd::log_lines(&format!("指令（{comp_name}）"), &spec, &outcome) {
                push_log(st, line);
            }
            if miedit_core::runcmd::should_abort(&spec, &outcome) {
                return Err(miedit_core::runcmd::abort_reason(&spec, &outcome));
            }
            Ok(())
        }
        miedit_core::micmd::MiInstruction::WriteReg { key, name, value, r#type } => {
            let key = ex(key);
            let name = ex(name);
            let value = ex(value);
            let ty = r#type.trim().to_ascii_lowercase();
            let (pm, rest) = miedit_core::winutil::split_hive(&key)
                .map_err(|e| format!("注册表键无效 {key}: {e:#}"))?;
            let view = miedit_core::winutil::RegView::Default;
            let resolved = match ty.as_str() {
                "" | "str" => vec![(name.clone(), miedit_core::winutil::RegValue::Str(value.clone()))],
                "dword" => {
                    let d = miedit_core::regspec::parse_dword(&value)
                        .map_err(|e| format!("DWORD 值无效 {name}={value}: {e}"))?;
                    vec![(name.clone(), miedit_core::winutil::RegValue::Dword(d))]
                }
                other => return Err(format!("write_reg.type \"{other}\" 无效（str / dword）")),
            };
            // 精确清理：安装前键不存在 → 卸载可整树删除；否则只删写入的值
            let created_key = !miedit_core::winutil::registry_key_exists(pm, &rest, view);
            miedit_core::winutil::registry_create_set_view(pm, &rest, &resolved, view)
                .map_err(|e| format!("写注册表失败 {key}\\{name}: {e:#}"))?;
            if !effects.registry_keys.contains(&key) {
                effects.registry_keys.push(key.clone());
            }
            let r = miedit_core::manifest::RegistryValueRef {
                key: key.clone(),
                name: name.clone(),
                created_key,
            };
            if !effects.registry_values.contains(&r) {
                effects.registry_values.push(r);
            }
            Ok(())
        }
        miedit_core::micmd::MiInstruction::CreateDir { path } => {
            // 文件系统路径归一化：NSIS 风格 `\` → 平台分隔符（与解包链路
            // `entry.path.replace('\\', "/")` 同口径；Linux 实机适配，
            // Windows 上 MAIN_SEPARATOR 本就是 `\`，行为不变）。
            // 注意：注册表键路径的 `\` 是 hive 分隔符，不在此归一化范围。
            let pb = PathBuf::from(ex(path).replace('\\', std::path::MAIN_SEPARATOR_STR).trim());
            fs::create_dir_all(&pb)
                .map_err(|e| format!("创建目录失败 {}: {e}", pb.display()))?;
            // 安装目录内 → 记相对路径（复用提取期 record_dir 逐级登记）；
            // 目录外 → 记绝对路径 extra_dirs（卸载时 NSIS RMDir 语义清理）
            match pb.strip_prefix(dest) {
                Ok(rel) => {
                    let rel = rel.to_string_lossy().replace('\\', "/");
                    if !rel.is_empty() {
                        record_dir(dirs, seen_dirs, &rel);
                    }
                }
                Err(_) => {
                    let abs = pb.display().to_string();
                    if !effects.extra_dirs.contains(&abs) {
                        effects.extra_dirs.push(abs);
                    }
                }
            }
            Ok(())
        }
        miedit_core::micmd::MiInstruction::DeleteFile { path } => {
            let pb = PathBuf::from(ex(path).replace('\\', std::path::MAIN_SEPARATOR_STR).trim());
            match fs::remove_file(&pb) {
                Ok(()) => push_log(st, format!("已删除: {}", pb.display())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    // NSIS Delete：缺失静默跳过
                }
                Err(e) => return Err(format!("删除失败 {}: {e}", pb.display())),
            }
            Ok(())
        }
        miedit_core::micmd::MiInstruction::Rename { from, to } => {
            let from = PathBuf::from(ex(from).replace('\\', std::path::MAIN_SEPARATOR_STR).trim());
            let to = PathBuf::from(ex(to).replace('\\', std::path::MAIN_SEPARATOR_STR).trim());
            // NSIS Rename：目标已存在则失败（不覆盖既有目标）
            if to.exists() {
                return Err(format!(
                    "重命名失败：目标已存在 {} → {}",
                    from.display(),
                    to.display()
                ));
            }
            fs::rename(&from, &to).map_err(|e| {
                format!("重命名失败 {} → {}: {e}", from.display(), to.display())
            })?;
            push_log(st, format!("已重命名: {} → {}", from.display(), to.display()));
            Ok(())
        }
        miedit_core::micmd::MiInstruction::CopyFiles { from, to } => {
            let from = PathBuf::from(ex(from).replace('\\', std::path::MAIN_SEPARATOR_STR).trim());
            let to = PathBuf::from(ex(to).replace('\\', std::path::MAIN_SEPARATOR_STR).trim());
            let n = miedit_core::files::copy_tree(&from, &to)
                .map_err(|e| format!("复制失败 {} → {}: {e:#}", from.display(), to.display()))?;
            push_log(st, format!("已复制: {} → {}（{n} 字节）", from.display(), to.display()));
            Ok(())
        }
        miedit_core::micmd::MiInstruction::WriteIni { file, section, key, value } => {
            let file = PathBuf::from(ex(file).replace('\\', std::path::MAIN_SEPARATOR_STR).trim());
            let section = ex(section);
            let key = ex(key);
            let value = ex(value);
            // 读 → 纯函数改写（缺失文件视为空内容，即新建）→ 写回
            let content = match fs::read_to_string(&file) {
                Ok(c) => c,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                Err(e) => {
                    return Err(format!("读取 INI 失败 {}: {e}", file.display()))
                }
            };
            let new_content =
                miedit_core::micmd::write_ini_str(&content, &section, &key, &value);
            fs::write(&file, new_content)
                .map_err(|e| format!("写入 INI 失败 {}: {e}", file.display()))?;
            push_log(
                st,
                format!("已写入 INI: {} [{}] {}={}", file.display(), section, key, value),
            );
            Ok(())
        }
        miedit_core::micmd::MiInstruction::Sleep { ms } => {
            std::thread::sleep(std::time::Duration::from_millis(*ms));
            Ok(())
        }
        // DetailPrint 已在上层输出（纯日志不执行）
        miedit_core::micmd::MiInstruction::DetailPrint { .. } => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 步骤 30：CRC 后台校验门——安装线程在提取前等待校验完成
    #[test]
    fn crc_gate_pending_then_ok() {
        let gate = Arc::new(CrcGate::new_pending());
        let g2 = gate.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(30));
            g2.complete(Ok(()));
        });
        // wait_ok 阻塞至后台完成（30ms 内返回 Ok）
        gate.wait_ok(None).unwrap();
    }

    #[test]
    fn crc_gate_failed_blocks_install() {
        let gate = CrcGate::ready(Err("安装包已损坏（CRC32 校验失败: payload 分区）".into()));
        let err = gate
            .wait_ok(None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("CRC32"), "失败原因应透传: {err}");
        assert_eq!(gate.failed_msg().as_deref().unwrap(), "安装包已损坏（CRC32 校验失败: payload 分区）");
    }

    #[test]
    fn crc_gate_ready_passes_immediately() {
        let gate = CrcGate::ready(Ok(()));
        gate.wait_ok(None).unwrap();
        assert!(gate.failed_msg().is_none());
    }

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
