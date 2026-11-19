//! 深色/浅色主题模式（编辑器与构建器外观，路线图步骤 44）。
//!
//! - [`ThemeMode`]：`跟随系统`（默认）/`深色`/`浅色`，serde 持久化到
//!   构建器设置文件（[`BuilderSettings`]）；
//! - [`resolve`]：模式 + 系统当前偏好 → 实际是否深色（纯函数，单测覆盖）；
//! - [`system_prefers_dark`]：系统偏好探测（Windows 读注册表
//!   `AppsUseLightTheme`，与 NSIS/Inno 无关但与 VSCode 惯例一致；
//!   Linux 依次尝试 gsettings / GTK_THEME / 默认深色）。

use serde::{Deserialize, Serialize};

/// 主题模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    /// 跟随系统（默认）
    #[default]
    System,
    /// 强制深色
    Dark,
    /// 强制浅色
    Light,
}

impl ThemeMode {
    /// 中文显示名（设置菜单用）。
    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::System => "跟随系统",
            ThemeMode::Dark => "深色",
            ThemeMode::Light => "浅色",
        }
    }
}

/// 模式解析：`system_dark` 为系统当前是否深色。
/// 纯函数：System → `system_dark`；Dark → true；Light → false。
pub fn resolve(mode: ThemeMode, system_dark: bool) -> bool {
    match mode {
        ThemeMode::System => system_dark,
        ThemeMode::Dark => true,
        ThemeMode::Light => false,
    }
}

/// 构建器本地设置（与脚本 TOML 解耦：主题是应用级偏好，不属于项目）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BuilderSettings {
    /// 主题模式（默认跟随系统）
    #[serde(default)]
    pub theme: ThemeMode,
}

impl BuilderSettings {
    /// 从 JSON 文件读取（文件缺失/损坏 → 默认值，永不阻塞启动）。
    pub fn load_from(path: &std::path::Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// 写入 JSON 文件（尽力而为，失败返回错误供上层提示）。
    pub fn save_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).unwrap_or_default())
    }
}

/// 构建器设置文件路径（跨平台）：
/// Windows `%APPDATA%\MIEdit\builder.json`；其余
/// `$XDG_CONFIG_HOME|~/.config` 下 `miedit/builder.json`。
/// 环境变量缺失时返回 None（上层退回默认值）。
pub fn settings_path() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        std::env::var("APPDATA")
            .ok()
            .map(|base| std::path::PathBuf::from(base).join("MIEdit").join("builder.json"))
    }
    #[cfg(not(windows))]
    {
        let base = match std::env::var("XDG_CONFIG_HOME") {
            Ok(v) if !v.trim().is_empty() => std::path::PathBuf::from(v),
            _ => {
                let home = std::env::var("HOME").ok()?;
                std::path::PathBuf::from(home).join(".config")
            }
        };
        Some(base.join("miedit").join("builder.json"))
    }
}

/// 系统当前是否偏好深色（尽力而为，失败 → true 深色兜底）。
///
/// - Windows：`HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize`
///   的 `AppsUseLightTheme`（DWORD，1 = 浅色）；
/// - Linux：优先 `gsettings get org.gnome.desktop.interface color-scheme`
///   （`prefer-dark` → 深色），其次 `GTK_THEME` 含 `-dark`；默认深色。
#[cfg(windows)]
pub fn system_prefers_dark() -> bool {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    match hkcu.open_subkey_with_flags(
        r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
        KEY_READ,
    ) {
        Ok(k) => k
            .get_value::<u32, _>("AppsUseLightTheme")
            .map(|v| v != 1)
            .unwrap_or(true),
        Err(_) => true,
    }
}

/// 非 Windows 平台的系统主题探测（见 [`system_prefers_dark`]）。
#[cfg(not(windows))]
pub fn system_prefers_dark() -> bool {
    if let Ok(out) = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "color-scheme"])
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            if s.contains("prefer-dark") {
                return true;
            }
            if s.contains("prefer-light") {
                return false;
            }
        }
    }
    if let Ok(t) = std::env::var("GTK_THEME") {
        if t.to_ascii_lowercase().contains("-dark") {
            return true;
        }
        if t.to_ascii_lowercase().contains("-light") {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve() {
        assert!(resolve(ThemeMode::System, true));
        assert!(!resolve(ThemeMode::System, false));
        assert!(resolve(ThemeMode::Dark, false));
        assert!(!resolve(ThemeMode::Light, true));
    }

    #[test]
    fn test_theme_mode_serde_roundtrip() {
        for m in [ThemeMode::System, ThemeMode::Dark, ThemeMode::Light] {
            let j = serde_json::to_string(&m).unwrap();
            let back: ThemeMode = serde_json::from_str(&j).unwrap();
            assert_eq!(back, m);
        }
        assert_eq!(serde_json::to_string(&ThemeMode::System).unwrap(), "\"system\"");
    }

    #[test]
    fn test_labels() {
        assert_eq!(ThemeMode::System.label(), "跟随系统");
        assert_eq!(ThemeMode::Dark.label(), "深色");
        assert_eq!(ThemeMode::Light.label(), "浅色");
    }

    #[test]
    fn test_builder_settings_roundtrip() {
        let dir = std::env::temp_dir().join(format!("miedit-theme-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("builder.json");
        let s = BuilderSettings { theme: ThemeMode::Light };
        s.save_to(&p).unwrap();
        assert_eq!(BuilderSettings::load_from(&p), s);
        // 损坏文件 → 默认值
        std::fs::write(&p, "{oops").unwrap();
        assert_eq!(BuilderSettings::load_from(&p), BuilderSettings::default());
        // 缺失文件 → 默认值
        std::fs::remove_file(&p).unwrap();
        assert_eq!(BuilderSettings::load_from(&p), BuilderSettings::default());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_settings_path_is_some() {
        // Linux 沙箱有 HOME；Windows 侧 CI 有 APPDATA。两者必有其一。
        #[cfg(not(windows))]
        assert!(settings_path().is_some());
    }

    #[test]
    fn test_system_prefers_dark_returns() {
        // 探测不 panic 且有返回值（沙箱无桌面环境时走默认深色）
        let _ = system_prefers_dark();
    }
}
