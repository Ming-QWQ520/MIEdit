//! 安装目录解析与校验（NSIS DirectoryPage 语义对齐，路线图步骤 20）。
//!
//! NSIS 的目录页在用户改动路径时**动态刷新**所选驱动器的可用空间，
//! 并在路径无效时阻止前进；`AllowRootDirInstall`（默认 false）额外禁止
//! 把程序安装到盘根或网络共享根。本模块把这些决策做成纯字符串逻辑，
//! 供安装器 UI 在每次输入变化时调用（不触碰文件系统，可跨平台单测）：
//!
//! - [`parse_dir_text`]：清洗输入框文本（空白/包裹引号），`C:` 规范化为 `C:\`；
//! - [`drive_root_of`]：提取盘根（`C:\foo` → `C:\`）或 UNC 共享根；
//! - [`is_root_path`]：是否为盘根/共享根（`AllowRootDirInstall` 判定口径）；
//! - [`classify_inst_dir`]：目录校验（空 / 无效 / 根目录拒绝）。

// ---------------------------------------------------------------------------
// 输入清洗
// ---------------------------------------------------------------------------

/// 清洗目录输入框文本：去首尾空白与**成对包裹**的引号；仅输入 `C:`
/// 这类裸盘符时补全为 `C:\`（NSIS 目录页的规范形态）。清洗后为空返回 None。
pub fn parse_dir_text(text: &str) -> Option<String> {
    let mut t = text.trim();
    // 成对引号剥两层（浏览对话框不会产生，主要覆盖手工粘贴场景）
    for _ in 0..2 {
        let trimmed = t.trim().trim_start_matches('"').trim_end_matches('"').trim();
        if trimmed.len() == t.len() {
            break;
        }
        t = trimmed;
    }
    if t.is_empty() {
        return None;
    }
    // 裸盘符 "C:" → "C:\"（大小写保留）
    let b = t.as_bytes();
    if b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return Some(format!("{}\\", t));
    }
    Some(t.to_string())
}

// ---------------------------------------------------------------------------
// 根路径判定
// ---------------------------------------------------------------------------

/// 提取路径的"根"：盘符路径 → `C:\`；UNC 路径 → `\\server\share`；
/// 相对路径 / 空 → None。不访问文件系统。
pub fn drive_root_of(path: &str) -> Option<String> {
    let p = path.trim();
    if let Some(rest) = p.strip_prefix(r"\\") {
        let mut it = rest.split(['\\', '/']).filter(|s| !s.is_empty());
        let server = it.next()?;
        let share = it.next()?;
        return Some(format!(r"\\{}\{}", server, share));
    }
    let b = p.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return Some(format!("{}\\", &p[..2]));
    }
    None
}

/// 是否为盘根（`C:\`、`C:`）或 UNC 共享根（`\\srv\share`）。
/// NSIS `AllowRootDirInstall` 的"根目录"判定口径：盘根与网络共享根都算根。
pub fn is_root_path(path: &str) -> bool {
    let Some(p) = parse_dir_text(path) else {
        return false;
    };
    if let Some(rest) = p.strip_prefix(r"\\") {
        // \\server\share 后不再有PathComponent 即为共享根
        return rest
            .split(['\\', '/'])
            .filter(|s| !s.is_empty())
            .skip(2)
            .next()
            .is_none();
    }
    let b = p.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        // "X:" 之后仅剩分隔符（或没有）即盘根
        return b[2..]
            .iter()
            .all(|&c| c == b'\\' || c == b'/');
    }
    false
}

// ---------------------------------------------------------------------------
// 目录校验（纯字符串逻辑；盘是否存在由调用方用文件系统补充检查）
// ---------------------------------------------------------------------------

/// 目录校验结果（NSIS 目录页对无效路径的处理：提示并停留当前页）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirIssue {
    /// 输入为空
    Empty,
    /// 无盘符/UNC 前缀的相对路径，或含 Windows 非法字符
    Invalid,
    /// 根目录安装且未允许（NSIS `AllowRootDirInstall` 默认语义）
    RootForbidden,
}

/// 校验结果 → 文案键（`dir_invalid` / `dir_root_warning`；Empty 同样显示无效提示）。
pub fn dir_issue_key(issue: DirIssue) -> &'static str {
    match issue {
        DirIssue::Empty | DirIssue::Invalid => "dir_invalid",
        DirIssue::RootForbidden => "dir_root_warning",
    }
}

/// Windows 路径非法字符：盘符冒号之外的 `:` 与 `<>|?*"`（`/` 允许，Windows API 接受）。
fn has_illegal_chars(path: &str) -> bool {
    let b = path.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'<' | b'>' | b'|' | b'?' | b'*' | b'"' => return true,
            b':' if !(i == 1 && b[0].is_ascii_alphabetic()) => return true,
            _ => {}
        }
    }
    false
}

/// 校验安装目录（纯字符串逻辑，不含文件系统检查）：
/// - `Err(Empty)`：清洗后为空；
/// - `Err(Invalid)`：相对路径（无 `X:` / `\\` 前缀）或含非法字符；
/// - `Err(RootForbidden)`：盘根/共享根安装且 `allow_root = false`（NSIS 默认）；
/// - `Ok(())`：通过（是否为已存在盘上的路径由调用方以 `drive_root_of(..).exists()` 补充）。
pub fn classify_inst_dir(text: &str, allow_root: bool) -> Result<(), DirIssue> {
    let Some(path) = parse_dir_text(text) else {
        return Err(DirIssue::Empty);
    };
    if has_illegal_chars(&path) {
        return Err(DirIssue::Invalid);
    }
    if drive_root_of(&path).is_none() {
        return Err(DirIssue::Invalid);
    }
    if is_root_path(&path) && !allow_root {
        return Err(DirIssue::RootForbidden);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_dir_text_basic() {
        assert_eq!(parse_dir_text("  C:\\App  ").as_deref(), Some(r"C:\App"));
        assert_eq!(parse_dir_text(r#""D:\My App""#).as_deref(), Some(r"D:\My App"));
        assert_eq!(parse_dir_text(r#""  E:\x  ""#).as_deref(), Some(r"E:\x"));
        assert_eq!(parse_dir_text(""), None);
        assert_eq!(parse_dir_text("   "), None);
        assert_eq!(parse_dir_text(r#"""#), None);
        assert_eq!(parse_dir_text(r#""""#), None);
    }

    #[test]
    fn test_parse_dir_text_bare_drive_normalized() {
        assert_eq!(parse_dir_text("C:").as_deref(), Some(r"C:\"));
        assert_eq!(parse_dir_text("d:").as_deref(), Some(r"d:\"));
        assert_eq!(parse_dir_text(" C: ").as_deref(), Some(r"C:\"));
        // 非"字母+冒号"的两字符输入不规范化
        assert_eq!(parse_dir_text("1:").as_deref(), Some("1:"));
    }

    #[test]
    fn test_drive_root_of() {
        assert_eq!(drive_root_of(r"C:\foo\bar").as_deref(), Some(r"C:\"));
        assert_eq!(drive_root_of("z:").as_deref(), Some(r"z:\"));
        assert_eq!(drive_root_of(r"\\srv\share\sub").as_deref(), Some(r"\\srv\share"));
        assert_eq!(drive_root_of(r"\\srv\share").as_deref(), Some(r"\\srv\share"));
        assert_eq!(drive_root_of("relative\\path"), None);
        assert_eq!(drive_root_of(""), None);
    }

    #[test]
    fn test_is_root_path() {
        assert!(is_root_path(r"C:\"));
        assert!(is_root_path("C:"));
        assert!(is_root_path("D:/"));
        assert!(is_root_path(r"\\srv\share"));
        assert!(is_root_path(r"\\srv\share\"));
        assert!(!is_root_path(r"C:\Program Files\App"));
        assert!(!is_root_path(r"\\srv\share\sub"));
        assert!(!is_root_path("relative"));
    }

    #[test]
    fn test_classify_ok_paths() {
        assert_eq!(classify_inst_dir(r"C:\Program Files\App", false), Ok(()));
        assert_eq!(classify_inst_dir(r"D:\Tools\app\", true), Ok(()));
        // 斜杠混用允许（Windows API 接受正斜杠）
        assert_eq!(classify_inst_dir("D:/Tools/app", false), Ok(()));
        // 允许根目录时根路径通过
        assert_eq!(classify_inst_dir("C:\\", true), Ok(()));
    }

    #[test]
    fn test_classify_empty_and_relative() {
        assert_eq!(classify_inst_dir("", false), Err(DirIssue::Empty));
        assert_eq!(classify_inst_dir("   ", false), Err(DirIssue::Empty));
        // 相对路径（无盘符）拒绝
        assert_eq!(classify_inst_dir("relative\\path", false), Err(DirIssue::Invalid));
        assert_eq!(classify_inst_dir("\\bare-unc-less", false), Err(DirIssue::Invalid));
    }

    #[test]
    fn test_classify_illegal_chars() {
        assert_eq!(classify_inst_dir(r"C:\fo|o", false), Err(DirIssue::Invalid));
        assert_eq!(classify_inst_dir(r#"C:\fo"o"#, false), Err(DirIssue::Invalid));
        assert_eq!(classify_inst_dir(r"C:\<x>", false), Err(DirIssue::Invalid));
        // 盘符位置之外的冒号非法
        assert_eq!(classify_inst_dir(r"C:\foo:bar", false), Err(DirIssue::Invalid));
        assert_eq!(classify_inst_dir(r"\\srv\sha?re\f", false), Err(DirIssue::Invalid));
    }

    #[test]
    fn test_classify_root_forbidden() {
        assert_eq!(classify_inst_dir("C:\\", false), Err(DirIssue::RootForbidden));
        assert_eq!(classify_inst_dir("C:", false), Err(DirIssue::RootForbidden));
        assert_eq!(classify_inst_dir(r"\\srv\share", false), Err(DirIssue::RootForbidden));
        assert_eq!(classify_inst_dir(r"\\srv\share", true), Ok(()));
    }

    #[test]
    fn test_dir_issue_key() {
        assert_eq!(dir_issue_key(DirIssue::Empty), "dir_invalid");
        assert_eq!(dir_issue_key(DirIssue::Invalid), "dir_invalid");
        assert_eq!(dir_issue_key(DirIssue::RootForbidden), "dir_root_warning");
    }
}
