//! 安装脚本（TOML）与安装包内嵌配置（JSON）共用的数据结构。
//!
//! 脚本中的 `background` / `license` / `icon` 是源文件路径，
//! 编译器读取内容后会将其清空并把数据内嵌（文本进 `license_text`，
//! 图片字节进包尾对应分区）。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn default_window_size() -> [f32; 2] {
    [960.0, 640.0]
}
fn default_accent() -> String {
    "#3B82F6".into()
}
fn default_bg_style() -> String {
    "cover".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub app: AppConfig,
    pub install: InstallConfig,
    pub ui: UiConfig,
    pub shortcuts: ShortcutsConfig,
    pub files: Vec<FileSpec>,
    pub registry: Vec<RegistrySpec>,
    /// 安装组件（NSIS Section / Inno Components 风格）；为空 = 不显示组件页，全部安装
    pub components: Vec<ComponentSpec>,
    /// 编译器生成：每条 [[files]] 规则展开出的文件（安装器按组件过滤用），脚本中无需手写
    pub files_map: Vec<Vec<FileRef>>,
}

/// 规则展开出的单个文件引用（编译器生成）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRef {
    pub path: String,
    pub size: u64,
    /// 源文件修改时间（Unix 秒）；旧包缺省为 0（未知），`ifnewer` 按需覆盖
    #[serde(default)]
    pub mtime: u64,
}

impl Config {
    /// 卸载注册表键名（`...\Uninstall\{app_id}`），未填写则按名称生成。
    pub fn app_id_or_slug(&self) -> String {
        if !self.app.app_id.trim().is_empty() {
            self.app.app_id.trim().to_string()
        } else {
            crate::util::slugify(&self.app.name)
        }
    }

    /// 是否显示许可协议页面。
    pub fn license_shown(&self) -> bool {
        let has_text = self
            .ui
            .license_text
            .as_deref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        has_text && self.ui.show_license_page.unwrap_or(true)
    }

    /// 是否显示组件选择页面。
    pub fn components_shown(&self) -> bool {
        !self.components.is_empty() && self.ui.show_components_page.unwrap_or(true)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// 应用显示名称
    pub name: String,
    pub version: String,
    pub publisher: String,
    /// 卸载注册表键名，留空由编译器按名称生成
    pub app_id: String,
    /// 主程序在包内的相对路径（用于快捷方式与 DisplayIcon），可为空
    pub exe: String,
    /// 应用程序网站（写入注册表 URLInfoAbout），可为空
    pub url: String,
    /// 应用图标（.ico，仅编译期用于替换成品 EXE 图标与安装窗口图标）
    pub icon: Option<String>,
    /// true = 每机器安装（HKLM + 公共开始菜单，需要管理员）；false = 每用户安装（HKCU）
    pub per_machine: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct InstallConfig {
    /// 默认安装目录模板：
    /// `{autopf}` `{autopf86}` `{localappdata}` `{appdata}` `{desktop}` `{docs}` `{app_name}`
    pub dir_template: String,
    /// 安装完成后可运行的程序（包内相对路径），留空则用 `app.exe`；
    /// 若 `run_tasks` 非空则忽略此字段
    pub run_after: String,
    /// 安装完成后可选运行的任务（Inno [Run] 风格，完成页显示为复选框）
    pub run_tasks: Vec<RunTask>,
    /// 用户数据/配置目录模板（如 `{appdata}\\{app_name}`）：
    /// 安装时解析并写入卸载清单，安装时不删除；卸载确认页可勾选删除
    pub user_data_dirs: Vec<String>,
    /// 压缩方式：`deflate`（条目级）或 `solid`（NSIS 风格整包压缩）
    pub compression: String,
    /// 文件覆盖策略（NSIS SetOverwrite）：`on`（默认）| `off` | `try` | `ifnewer`
    pub overwrite: String,
    /// 是否在安装目录写入 install.log（NSIS LogSet 风格），默认开启
    pub write_log: bool,
    /// 卸载程序图标（.ico，可选；缺省使用 app.icon）
    pub uninstall_icon: Option<String>,
}

impl Default for InstallConfig {
    fn default() -> Self {
        Self {
            dir_template: "{autopf}\\{app_name}".into(),
            run_after: String::new(),
            run_tasks: Vec::new(),
            user_data_dirs: Vec::new(),
            compression: "deflate".into(),
            overwrite: "on".into(),
            write_log: true,
            uninstall_icon: None,
        }
    }
}

/// 完成页的可选运行任务（Inno [Run] 风格）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RunTask {
    /// 完成页复选框文案（支持 {app} {version}）
    pub name: String,
    /// 包内相对路径（exe 用主程序打开；md/txt 等用系统默认程序打开）
    pub exe: String,
    /// 启动参数（仅 exe）
    pub args: String,
    /// 默认是否勾选
    pub default_checked: bool,
}

impl Default for RunTask {
    fn default() -> Self {
        Self {
            name: String::new(),
            exe: String::new(),
            args: String::new(),
            default_checked: true,
        }
    }
}

/// 安装组件（NSIS Section / Inno Components 风格）。
/// `files` 引用 `[[files]]` 规则索引（从 0 开始）；
/// 未被任何组件引用的规则始终安装。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ComponentSpec {
    pub name: String,
    pub description: String,
    /// 必装组件：勾选锁定
    pub required: bool,
    pub files: Vec<usize>,
}

impl Default for ComponentSpec {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            required: false,
            files: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    /// 安装窗口尺寸（无边框）
    pub window_size: [f32; 2],
    /// 背景图源路径（编译时读取并内嵌，之后清空）
    pub background: Option<String>,
    /// 内嵌背景图格式（png/jpg/...），由编译器填写
    pub background_format: Option<String>,
    /// cover | contain | stretch
    pub background_style: String,
    /// 主题色（十六进制，如 #3B82F6）
    pub accent: String,
    /// 许可文件源路径（编译时读取内嵌，之后清空）
    pub license: Option<String>,
    /// 内嵌许可文本（UTF-8）
    pub license_text: Option<String>,
    /// 是否显示许可页面；None = 自动（有许可文本则显示）
    pub show_license_page: Option<bool>,
    /// 许可页需滚动到底部才允许接受（Inno 风格）
    pub license_scroll_required: bool,
    /// 是否显示组件选择页；None = 自动（有组件定义则显示）
    pub show_components_page: Option<bool>,
    /// 是否显示安装目录选择页
    pub show_dir_page: bool,
    /// UI 文案覆盖（键值见 README）
    pub text: BTreeMap<String, String>,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            window_size: default_window_size(),
            background: None,
            background_format: None,
            background_style: default_bg_style(),
            accent: default_accent(),
            license: None,
            license_text: None,
            show_license_page: None,
            license_scroll_required: false,
            show_components_page: None,
            show_dir_page: true,
            text: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ShortcutsConfig {
    pub start_menu: bool,
    pub desktop: bool,
    /// 快捷方式名称（默认用应用名）
    pub name: Option<String>,
    /// 主程序启动参数
    pub run_args: String,
}

impl Default for ShortcutsConfig {
    fn default() -> Self {
        Self {
            start_menu: true,
            desktop: false,
            name: None,
            run_args: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileSpec {
    /// 源文件 / 目录 / 通配符（相对脚本目录），支持 `*` `?` `**`
    pub source: String,
    /// 包内目标子目录（相对安装根目录），空 = 根目录
    #[serde(default)]
    pub dest: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RegistrySpec {
    /// 如 `HKCU\Software\Vendor\App` 或 `HKLM\...`
    pub key: String,
    /// 仅支持字符串值；`{install_dir}` 会在安装时替换
    pub values: BTreeMap<String, String>,
}

/// 读取 UI 文案：优先使用脚本覆盖，否则使用内置英文默认值。
pub fn ui_text(cfg: &Config, key: &str) -> String {
    if let Some(v) = cfg.ui.text.get(key) {
        return v.clone();
    }
    match default_text(key) {
        Some(s) => s.to_string(),
        None => key.to_string(),
    }
}

/// 内置默认 UI 文案表（英文）；未知键返回 None。
pub fn default_text(key: &str) -> Option<&'static str> {
    const DEFAULT_TEXTS: &[(&str, &str)] = &[
        ("window_title", "{app} Setup"),
        ("welcome_chip", "WELCOME"),
        ("welcome_body", "Setup will install {app} {version} on your computer.\nIt is recommended to close other applications before continuing."),
        ("next", "Next"),
        ("back", "Back"),
        ("cancel", "Cancel"),
        ("install_now", "Install"),
        ("browse", "Browse..."),
        ("dir_title", "Choose Install Location"),
        ("dir_desc", "Setup will install {app} into the following folder.\nClick Install to begin, or Browse to change the folder."),
        ("license_title", "License Agreement"),
        ("license_desc", "Please read the following license agreement carefully."),
        ("license_accept", "I accept the agreement"),
        ("installing", "Installing {app}"),
        ("preparing", "Preparing to install..."),
        ("extracting", "Extracting: {file}"),
        ("finalizing", "Writing uninstaller and shortcuts..."),
        ("finish_title", "Installation Complete"),
        ("finish_body", "{app} {version} has been installed to:"),
        ("run_app", "Run {app}"),
        ("done_btn", "Finish"),
        ("error_title", "Installation Failed"),
        ("exit_body", "Setup is not finished yet. Quit setup anyway?"),
        ("disk_space", "Estimated required space: {size}"),
        ("components_title", "Select Components"),
        ("components_desc", "Choose which components of {app} to install."),
        ("components_select_all", "Select All"),
        ("components_reset", "Reset"),
        ("components_required", "Required"),
        ("free_space", "Free space on drive: {free}"),
        ("app_running_title", "App Is Running"),
        ("app_running_body", "{app} appears to be running. Please close it before continuing, or retry the check."),
        ("retry", "Retry"),
        ("continue_anyway", "Continue Anyway"),
        ("upgrade_notice", "Found installed version {old}; it will be upgraded to {new}."),
        // 步骤 8：文件写入错误模态（NSIS 文件错误对话框：重试/忽略/中止）
        ("file_error_title", "写入文件失败"),
        ("file_error_body", "无法写入以下文件：\n{file}\n\n{error}"),
        (
            "file_error_hint",
            "请关闭正在使用该文件的程序后选择「重试」；选择「忽略」将跳过该文件并在系统重启后替换；选择「中止」将停止安装（已写入的文件保留）。",
        ),
        ("file_error_ignore", "忽略"),
        ("file_error_abort", "中止"),
        // 步骤 9：完成页"需要重启"琥珀提示
        (
            "finish_reboot_required",
            "需要重启以完成安装：{count} 个文件将在系统重启后替换。",
        ),
    ];
    DEFAULT_TEXTS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| *v)
}

/// 将文案中的 `{app}` / `{version}` 替换为实际值。
pub fn fill_template(s: &str, app: &str, version: &str) -> String {
    s.replace("{app}", app).replace("{version}", version)
}
