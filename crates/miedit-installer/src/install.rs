//! 后台安装线程：解包 payload → 写盘 → 调用 finalize 收尾。

use crate::finalize;
use crate::pkg::Package;
use anyhow::{Context as _, Result};
use eframe::egui;
use miedit_core::archive::ArchiveReader;
use std::collections::HashSet;
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

/// 启动后台安装线程，返回共享进度状态。
/// `selected_rules`：组件过滤后允许安装的 `[[files]]` 规则索引；None = 全部安装。
pub fn spawn(
    pkg: Arc<Package>,
    dest: PathBuf,
    ctx: Option<egui::Context>,
    selected_rules: Option<std::collections::HashSet<usize>>,
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
            let res = run_install(&pkg, &dest, &st2, ctx.as_ref(), selected_rules.as_ref());
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
) -> Result<()> {
    // 1. 旧版本检测（升级提示 + 静默卸载旧版）：新版为内嵌清单，旧版兼容 uninstall.json
    let old_exe = dest.join("uninstall.exe");
    let old_json = dest.join("uninstall.json");
    if old_exe.is_file() {
        let old_version: Option<String> =
            miedit_core::trailer::read_uninstall_data(&old_exe)
                .and_then(|(m, _)| serde_json::from_slice::<serde_json::Value>(&m).ok())
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
        let mut out = BufWriter::new(fs::File::create(&target).with_context(|| {
            format!("创建文件失败: {}", target.display())
        })?);
        out.write_all(&data)?;
        out.flush()?;
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
    finalize::finalize_install(pkg, dest, &files, &dirs, st)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    // 组件过滤契约：选中规则 → 展开为允许安装的包内路径集合
    #[test]
    fn test_component_filter_mapping() {
        use miedit_core::config::FileRef;
        let files_map = vec![
            vec![FileRef { path: "a.exe".into(), size: 1 }],
            vec![FileRef { path: "docs/x.md".into(), size: 1 }],
            vec![FileRef { path: "data/c.json".into(), size: 1 }],
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
