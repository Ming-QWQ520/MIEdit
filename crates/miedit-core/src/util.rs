//! 通用工具：标识符规范化、颜色解析、路径模板解析等。

use std::path::PathBuf;

/// 将应用名转换为注册表键用的短横线标识（只保留 ASCII 字母数字）。
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = true;
    for ch in name.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "App".into()
    } else {
        out
    }
}

/// 解析 `#RRGGBB` 十六进制颜色。
pub fn parse_hex_color(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some([r, g, b])
}

/// 字节数的人类可读表示。
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

fn env_or(key: &str, default: &str) -> String {
    match std::env::var(key) {
        Ok(v) if !v.trim().is_empty() => v,
        _ => default.to_string(),
    }
}

/// 解析安装目录模板。支持占位符：
/// `{autopf}` `{autopf86}` `{localappdata}` `{appdata}` `{desktop}` `{docs}` `{app_name}`
#[cfg(windows)]
pub fn resolve_path_template(tpl: &str, app_name: &str) -> PathBuf {
    let profile = env_or("USERPROFILE", "C:\\Users\\Public");
    let s = tpl
        .replace("{autopf}", &env_or("ProgramFiles", "C:\\Program Files"))
        .replace("{autopf86}", &env_or("ProgramFiles(x86)", "C:\\Program Files (x86)"))
        .replace(
            "{localappdata}",
            &env_or("LOCALAPPDATA", &format!("{profile}\\AppData\\Local")),
        )
        .replace("{appdata}", &env_or("APPDATA", &format!("{profile}\\AppData\\Roaming")))
        .replace("{desktop}", &format!("{profile}\\Desktop"))
        .replace("{docs}", &format!("{profile}\\Documents"))
        .replace("{app_name}", app_name);
    PathBuf::from(s)
}

/// 非 Windows 平台的路径模板解析（路线图步骤 21，Linux 适配）：
/// `{autopf}` → `/opt`（XDG 应用安装惯例）；`{autopf86}` → `/opt`；
/// `{localappdata}` / `{appdata}` → `$XDG_DATA_HOME`（缺省 `~/.local/share`）；
/// `{desktop}` → `$HOME/Desktop`；`{docs}` → `$HOME/Documents`；
/// `{app_name}` → 应用名。模板里的 `\` 统一转为 `/`（Windows 风格脚本
/// 在 Linux 上同样可用）。家目录取 `$HOME`（缺省 `/root`，root 兼容）。
#[cfg(not(windows))]
pub fn resolve_path_template(tpl: &str, app_name: &str) -> PathBuf {
    let home = env_or("HOME", "/root");
    let xdg_data = env_or("XDG_DATA_HOME", &format!("{home}/.local/share"));
    let s = tpl
        .replace("{autopf}", "/opt")
        .replace("{autopf86}", "/opt")
        .replace("{localappdata}", &xdg_data)
        .replace("{appdata}", &xdg_data)
        .replace("{desktop}", &format!("{home}/Desktop"))
        .replace("{docs}", &format!("{home}/Documents"))
        .replace("{app_name}", app_name)
        .replace('\\', "/");
    PathBuf::from(s)
}

/// 递归统计目录下的文件数与总字节数（卸载完成页统计用，路线图步骤 14）。
/// 尽力而为：无权限/消失的子项跳过；`path` 不存在返回 `(0, 0)`。
pub fn dir_file_stats(path: &std::path::Path) -> (usize, u64) {
    let (mut files, mut bytes) = (0usize, 0u64);
    let walk = walkdir::WalkDir::new(path).follow_links(false).into_iter();
    for entry in walk.flatten() {
        if entry.file_type().is_file() {
            files += 1;
            bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    (files, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("MIEdit Demo"), "miedit-demo");
        assert_eq!(slugify("我的 应用 App"), "app");
        assert_eq!(slugify("A---B"), "a-b");
        assert_eq!(slugify("!!!"), "App");
    }

    #[test]
    fn test_hex() {
        assert_eq!(parse_hex_color("#3B82F6"), Some([0x3b, 0x82, 0xf6]));
        assert_eq!(parse_hex_color("FF0000"), Some([255, 0, 0]));
        assert_eq!(parse_hex_color("zzzzzz"), None);
    }

    #[test]
    fn test_human_size() {
        assert_eq!(human_size(500), "500 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MB");
    }

    // Linux 适配（路线图步骤 21）：非 Windows 平台的模板解析
    #[cfg(not(windows))]
    #[test]
    fn test_resolve_path_template_linux() {
        std::env::remove_var("XDG_DATA_HOME");
        // Windows 风格反斜杠模板在 Linux 上同样可用（分隔符归一）
        let p = resolve_path_template("{autopf}\\{app_name}", "Demo");
        assert_eq!(p, PathBuf::from("/opt/Demo"));
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        let p = resolve_path_template("{localappdata}\\Programs\\{app_name}", "Demo");
        assert_eq!(p, PathBuf::from(format!("{home}/.local/share/Programs/Demo")));
        let p = resolve_path_template("{appdata}\\{app_name}", "Demo");
        assert_eq!(p, PathBuf::from(format!("{home}/.local/share/Demo")));
        let p = resolve_path_template("{desktop}\\{app_name}.lnk", "Demo");
        assert_eq!(p, PathBuf::from(format!("{home}/Desktop/Demo.lnk")));
        let p = resolve_path_template("{docs}\\{app_name}", "Demo");
        assert_eq!(p, PathBuf::from(format!("{home}/Documents/Demo")));
        let p = resolve_path_template("{autopf86}\\{app_name}", "Demo");
        assert_eq!(p, PathBuf::from("/opt/Demo"));
        // XDG_DATA_HOME 优先
        std::env::set_var("XDG_DATA_HOME", "/tmp/xdg-data-test");
        let p = resolve_path_template("{localappdata}\\{app_name}", "Demo");
        assert_eq!(p, PathBuf::from("/tmp/xdg-data-test/Demo"));
        std::env::remove_var("XDG_DATA_HOME");
    }

    #[test]
    fn test_dir_file_stats() {
        let base = std::env::temp_dir().join(format!("miedit-stats-test-{}", std::process::id()));
        let sub = base.join("nested");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(base.join("a.txt"), [0u8; 100]).unwrap();
        std::fs::write(sub.join("b.bin"), [0u8; 50]).unwrap();
        let (files, bytes) = dir_file_stats(&base);
        assert_eq!(files, 2,"应统计到两个文件");
        assert_eq!(bytes, 150);
        // 不存在的目录返回 (0, 0)
        let (f2, b2) = dir_file_stats(&base.join("missing"));
        assert_eq!((f2, b2), (0, 0));
        std::fs::remove_dir_all(&base).ok();
    }
}
