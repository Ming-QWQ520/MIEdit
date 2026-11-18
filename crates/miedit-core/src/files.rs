//! 文件收集：把脚本中的 `[[files]]` 规则展开为具体文件列表（供编译器打包）。
//!
//! - 源是文件：入包路径 = `dest/文件名`
//! - 源是目录：递归收录（含空目录），入包路径 = `dest/相对路径`
//! - 源是通配符（含 `*` / `?`）：从最后一个字面前缀目录开始递归匹配，
//!   入包路径 = `dest/相对路径`，支持 `**` 跨目录。

use crate::config::FileSpec;
use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct CollectedFile {
    pub fs_path: PathBuf,
    /// 包内路径（'/' 分隔，相对安装根目录）
    pub arc_path: String,
    pub size: u64,
    /// 源文件修改时间（Unix 秒；供 NSIS `SetOverwrite ifnewer` 比较）
    pub mtime: u64,
    pub is_dir: bool,
}

/// 读取文件修改时间（Unix 秒）；失败返回 0（未知）。
pub fn mtime_epoch(p: &Path) -> u64 {
    let t = match p.metadata() {
        Ok(m) => match m.modified() {
            Ok(t) => t,
            Err(_) => return 0,
        },
        Err(_) => return 0,
    };
    t.duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 展开一条文件规则。
pub fn gather(spec: &FileSpec, base_dir: &Path) -> Result<Vec<CollectedFile>> {
    let source = spec.source.trim();
    if source.is_empty() {
        bail!("[[files]] source 不能为空");
    }
    let dest = normalize_dest(&spec.dest);
    let src = base_dir.join(source);

    if source.contains('*') || source.contains('?') {
        gather_wildcard(source, &dest, base_dir)
    } else if src.is_file() {
        let name = src
            .file_name()
            .ok_or_else(|| anyhow!("无效文件路径: {source}"))?
            .to_string_lossy()
            .to_string();
        Ok(vec![CollectedFile {
            fs_path: src.clone(),
            arc_path: join_arc(&dest, &name),
            size: std::fs::metadata(&src).map(|m| m.len()).unwrap_or(0),
            mtime: mtime_epoch(&src),
            is_dir: false,
        }])
    } else if src.is_dir() {
        gather_dir(&src, &dest)
    } else {
        bail!("找不到 source: {}（相对 {}）", source, base_dir.display())
    }
}

fn gather_dir(src: &Path, dest: &str) -> Result<Vec<CollectedFile>> {
    let mut out = Vec::new();
    for entry in WalkDir::new(src).sort_by_file_name() {
        let entry = entry.with_context(|| format!("遍历目录失败: {}", src.display()))?;
        let rel = entry
            .path()
            .strip_prefix(src)
            .expect("walkdir 结果应位于源目录内");
        if rel.as_os_str().is_empty() {
            continue; // 跳过根自身
        }
        let arc = join_arc(dest, &rel.to_string_lossy().replace('\\', "/"));
        if entry.file_type().is_dir() {
            out.push(CollectedFile {
                fs_path: entry.path().to_path_buf(),
                arc_path: arc,
                size: 0,
                mtime: 0,
                is_dir: true,
            });
        } else {
            out.push(CollectedFile {
                fs_path: entry.path().to_path_buf(),
                arc_path: arc,
                size: entry.metadata().map(|m| m.len()).unwrap_or(0),
                mtime: mtime_epoch(entry.path()),
                is_dir: false,
            });
        }
    }
    Ok(out)
}

fn gather_wildcard(source: &str, dest: &str, base_dir: &Path) -> Result<Vec<CollectedFile>> {
    let normalized = source.replace('\\', "/");
    let comps: Vec<&str> = normalized
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    if comps.is_empty() {
        bail!("无效通配符: {source}");
    }
    // 最后一个字面前缀目录作为遍历根
    let mut literal = 0usize;
    while literal < comps.len()
        && !comps[literal].contains('*')
        && !comps[literal].contains('?')
    {
        literal += 1;
    }
    let mut root = base_dir.to_path_buf();
    for c in &comps[..literal] {
        root.push(c);
    }
    if !root.exists() {
        bail!("通配符前缀目录不存在: {}", root.display());
    }
    let rest = &comps[literal..];
    if rest.is_empty() {
        bail!("通配符必须包含 * 或 ?: {source}");
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for entry in WalkDir::new(&root).sort_by_file_name() {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let rel = match entry.path().strip_prefix(&root) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        let comps: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect();
        let comp_refs: Vec<&str> = comps.iter().map(|s| s.as_str()).collect();
        if !match_components(rest, &comp_refs) {
            continue;
        }
        let arc = join_arc(dest, &comps.join("/"));
        if entry.file_type().is_dir() {
            // 通配符匹配到的目录仅作为目录占位
            if seen.insert(arc.clone()) {
                out.push(CollectedFile {
                    fs_path: entry.path().to_path_buf(),
                    arc_path: arc,
                    size: 0,
                    mtime: 0,
                    is_dir: true,
                });
            }
        } else {
            out.push(CollectedFile {
                fs_path: entry.path().to_path_buf(),
                arc_path: arc,
                size: entry.metadata().map(|m| m.len()).unwrap_or(0),
                mtime: mtime_epoch(entry.path()),
                is_dir: false,
            });
        }
    }
    Ok(out)
}

/// 归一化 dest：'\'→'/'，去掉首尾 '/'。
pub fn normalize_dest(dest: &str) -> String {
    dest.replace('\\', "/")
        .split('/')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

fn join_arc(dest: &str, rel: &str) -> String {
    let rel = rel.trim_start_matches('/');
    if dest.is_empty() {
        rel.to_string()
    } else {
        format!("{dest}/{rel}")
    }
}

/// 校验包内路径安全（禁止绝对路径与 `..`）。
pub fn is_safe_arc_path(p: &str) -> bool {
    if p.is_empty() {
        return false;
    }
    if p.starts_with('/') || p.starts_with('\\') || p.contains(':') {
        return false;
    }
    !p.split(['/', '\\']).any(|c| c == "..")
}

// ---------------------------------------------------------------------------
// NSIS CopyFiles 指令的复制内核（路线图步骤 37）
// ---------------------------------------------------------------------------

/// 递归复制文件或整个目录树（NSIS `CopyFiles` 语义）。
///
/// - `src` 是文件：复制到 `dst`（`dst` 的父目录自动创建）
/// - `src` 是目录：递归复制整棵树到 `dst`（`dst` 作为目标根目录创建；
///   空目录照搬，同名文件覆盖——NSIS CopyFiles 覆盖语义）
/// - 返回复制的总字节数；`src` 不存在时报错（NSIS error flag 语义）
///
/// 防呆：`dst` 位于 `src` 内部时拒绝（会无限自嵌套复制）。
pub fn copy_tree(src: &Path, dst: &Path) -> Result<u64> {
    let meta = std::fs::metadata(src)
        .with_context(|| format!("复制源不存在或不可读: {}", src.display()))?;
    // 自嵌套防护：dst 的规范路径落在 src 内部则拒绝
    if meta.is_dir() {
        if let (Ok(src_c), Ok(dst_parent_c)) =
            (std::fs::canonicalize(src), std::fs::canonicalize(dst.parent().unwrap_or(dst)))
        {
            if dst_parent_c.starts_with(&src_c) {
                bail!(
                    "复制目标位于源目录内部（会无限自复制）: {} → {}",
                    src.display(),
                    dst.display()
                );
            }
        }
    }
    if meta.is_file() {
        if let Some(parent) = dst.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("创建目录失败: {}", parent.display()))?;
            }
        }
        let n = std::fs::copy(src, dst)
            .with_context(|| format!("复制文件失败: {} → {}", src.display(), dst.display()))?;
        return Ok(n);
    }
    copy_dir(src, dst)
}

/// 递归复制目录树（[`copy_tree`] 的目录分支），返回总字节数。
fn copy_dir(src: &Path, dst: &Path) -> Result<u64> {
    std::fs::create_dir_all(dst)
        .with_context(|| format!("创建目录失败: {}", dst.display()))?;
    let mut total = 0u64;
    for entry in std::fs::read_dir(src)
        .with_context(|| format!("读取目录失败: {}", src.display()))?
    {
        let entry = entry.with_context(|| format!("遍历目录失败: {}", src.display()))?;
        let sp = entry.path();
        let dp = dst.join(entry.file_name());
        if sp.is_dir() {
            total += copy_dir(&sp, &dp)?;
        } else if sp.is_file() {
            let n = std::fs::copy(&sp, &dp).with_context(|| {
                format!("复制文件失败: {} → {}", sp.display(), dp.display())
            })?;
            total += n;
        }
    }
    Ok(total)
}

// ---------------------------------------------------------------------------
// 安装期磁盘空间预检（路线图步骤 38）
// ---------------------------------------------------------------------------

/// 写文件所需的额外余量（防零头不足：目录项/簇分配/日志增长）。
pub const SPACE_RESERVE: u64 = 8 * 1024 * 1024;

/// 磁盘空间是否不足以写入该文件（NSIS 磁盘满中止语义的预检形态）。
///
/// 覆盖已有文件时可抵扣旧文件占用的空间（写入前旧文件已在盘上，
/// 新内容会替换它），因此判定为 `free + existing < new + RESERVE`。
/// `free` 不可知（`disk_free_space` 返回 None）时调用方跳过检查。
pub fn is_space_insufficient(free: u64, new_size: u64, existing_size: u64) -> bool {
    free.saturating_add(existing_size) < new_size.saturating_add(SPACE_RESERVE)
}

/// 空间不足的错误文案（进入失败页/静默日志；含期望与实际，NSIS
/// "Error writing to file / disk full" 的信息量对齐）。
pub fn space_error_text(free: u64, new_size: u64, path: &str) -> String {
    format!(
        "磁盘空间不足: 目标盘剩余 {free} 字节，写入 {path} 需要 {new_size} 字节（含 {}MB 余量），已中止安装",
        SPACE_RESERVE / (1024 * 1024)
    )
}

fn match_components(pat: &[&str], path: &[&str]) -> bool {
    if pat.is_empty() {
        return path.is_empty();
    }
    if pat[0] == "**" {
        for i in 0..=path.len() {
            if match_components(&pat[1..], &path[i..]) {
                return true;
            }
        }
        return false;
    }
    !path.is_empty() && match_component(pat[0], path[0]) && match_components(&pat[1..], &path[1..])
}

/// 单段通配符匹配（`*` 任意字符、`?` 单个字符，忽略大小写）。
fn match_component(pat: &str, s: &str) -> bool {
    fn rec(p: &[u8], s: &[u8]) -> bool {
        if p.is_empty() {
            return s.is_empty();
        }
        match p[0] {
            b'*' => (0..=s.len()).any(|i| rec(&p[1..], &s[i..])),
            b'?' => !s.is_empty() && rec(&p[1..], &s[1..]),
            c => !s.is_empty() && s[0].eq_ignore_ascii_case(&c) && rec(&p[1..], &s[1..]),
        }
    }
    rec(pat.as_bytes(), s.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_components() {
        assert!(match_components(&["*"], &["a.txt"]));
        assert!(match_components(&["*.png"], &["a.png"]));
        assert!(!match_components(&["*.png"], &["a.jpg"]));
        assert!(match_components(&["docs", "**", "*.md"], &["docs", "zh", "a.md"]));
        assert!(match_components(&["docs", "**", "*.md"], &["docs", "a.md"]));
        assert!(!match_components(&["docs", "**", "*.md"], &["other", "a.md"]));
        assert!(match_component("app*", "appv2"));
        assert!(match_component("???", "abc"));
        assert!(!match_component("???", "ab"));
    }

    #[test]
    fn test_safe_path() {
        assert!(is_safe_arc_path("app/bin/x.exe"));
        assert!(!is_safe_arc_path("../evil"));
        assert!(!is_safe_arc_path("a/../b"));
        assert!(!is_safe_arc_path("C:/abs"));
        assert!(!is_safe_arc_path(""));
    }

    #[test]
    fn test_join_arc() {
        assert_eq!(normalize_dest("a\\b\\"), "a/b");
        assert_eq!(join_arc("docs", "x.md"), "docs/x.md");
        assert_eq!(join_arc("", "x.md"), "x.md");
    }

    /// 建立唯一的临时目录（std 无临时目录助手；测试自清理）。
    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "miedit-core-{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn test_copy_tree_file_and_tree() {
        let root = temp_dir("copytree");
        let src = root.join("src");
        std::fs::create_dir_all(src.join("sub/deep")).unwrap();
        std::fs::write(src.join("a.txt"), "hello a").unwrap();
        std::fs::write(src.join("sub/b.bin"), &[0u8, 1, 2, 3]).unwrap();
        std::fs::create_dir_all(src.join("sub/empty")).unwrap();
        std::fs::write(src.join("sub/deep/c.txt"), "c").unwrap();

        // 文件复制：目标父目录自动创建
        let dst_file = root.join("out/dir/a.txt");
        let n = copy_tree(&src.join("a.txt"), &dst_file).unwrap();
        assert_eq!(n, 7);
        assert_eq!(std::fs::read_to_string(&dst_file).unwrap(), "hello a");

        // 目录树复制：空目录照搬、嵌套照搬、字节数累计
        let dst = root.join("out/tree");
        let total = copy_tree(&src, &dst).unwrap();
        assert_eq!(total, 7 + 4 + 1);
        assert_eq!(
            std::fs::read_to_string(dst.join("sub/deep/c.txt")).unwrap(),
            "c"
        );
        assert!(dst.join("sub/empty").is_dir());

        // 覆盖语义：同名文件被替换
        std::fs::write(src.join("a.txt"), "hello v2").unwrap();
        copy_tree(&src, &dst).unwrap();
        assert_eq!(
            std::fs::read_to_string(dst.join("a.txt")).unwrap(),
            "hello v2"
        );

        // 源不存在报错
        assert!(copy_tree(&root.join("nope"), &dst).is_err());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn test_copy_tree_rejects_self_nesting() {
        let root = temp_dir("selfnest");
        let src = root.join("data");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("x.txt"), "x").unwrap();
        // dst 在 src 内部：拒绝
        let err = copy_tree(&src, &src.join("inner")).unwrap_err();
        assert!(err.to_string().contains("无限自复制"), "{err}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn test_space_insufficient_and_text() {
        let m = 1024 * 1024u64;
        // 充足：free 远大于需求
        assert!(!is_space_insufficient(100 * m, 10 * m, 0));
        // 不足：free 小于 new+8MB
        assert!(is_space_insufficient(5 * m, 10 * m, 0));
        // 边界：恰好等于需求（free+existing == new+reserve）→ 视为足够
        assert!(!is_space_insufficient(10 * m + 8 * m, 10 * m, 0));
        // 覆盖抵扣：旧文件 6MB，free 13MB，new 10MB → 13+6=19 ≥ 10+8 ✓
        assert!(!is_space_insufficient(13 * m, 10 * m, 6 * m));
        // 抵扣不足：free 10MB + 旧 2MB < 10+8 → 不足
        assert!(is_space_insufficient(10 * m, 10 * m, 2 * m));
        // 溢出安全：u64 大值不 panic
        assert!(!is_space_insufficient(u64::MAX, u64::MAX / 2, u64::MAX / 2));
        let t = space_error_text(5, 100, "bin/app.exe");
        assert!(t.contains("磁盘空间不足") && t.contains("bin/app.exe"), "{t}");
    }
}
