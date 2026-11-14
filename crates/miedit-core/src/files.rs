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
}
