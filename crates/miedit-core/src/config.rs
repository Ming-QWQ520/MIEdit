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
    /// 安装类型（NSIS InstallTypes / Inno [Types] 风格）；为空 = 组件页不显示类型单选
    pub install_types: Vec<InstallTypeSpec>,
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
    /// 压缩方式（NSIS SetCompressor 对齐 + 扩展，全部纯 Rust）：
    /// `deflate`（条目级）| `solid`（整包 Deflate，NSIS solid 风格）| `lzma`（条目级）| 
    /// `lzma-solid`（整包 LZMA，NSIS 默认风格）| `brotli`（条目级）| `brotli-solid`（整包）
    pub compression: String,
    /// 文件覆盖策略（NSIS SetOverwrite）：`on`（默认）| `off` | `try` | `ifnewer`
    pub overwrite: String,
    /// 是否在安装目录写入 install.log（NSIS LogSet 风格），默认开启
    pub write_log: bool,
    /// 卸载程序图标（.ico，可选；缺省使用 app.icon）
    pub uninstall_icon: Option<String>,
    /// 卸载器壁纸（可选，源文件路径；编译期内嵌进卸载器 EXE 尾部，
    /// 卸载向导全窗口显示该图；缺省走内置渐变背景）
    pub uninstall_background: Option<String>,
    /// 最小系统版本门槛（如 "6.1" = Windows 7，"10.0" = Windows 10）；
    /// 空 = 不检查。低于门槛时安装器启动即报错退出（Inno MinVersion 对齐）
    pub min_version: String,
    /// 允许安装到盘根/网络共享根（NSIS `AllowRootDirInstall` 对齐，路线图步骤 20）。
    /// 默认 false：目录页拒绝根目录安装并提示选择子文件夹（NSIS 安全默认语义）
    pub allow_root_dir_install: bool,
    /// 安装前执行命令（NSIS `.onInit`/`Exec` 语义，路线图步骤 23）：
    /// 解包前执行，GUI 与静默模式同生效；`abort_on_error` 时中止安装
    pub exec_before: Vec<RunCommandSpec>,
    /// 安装完成后执行命令（NSIS `ExecWait` 语义，路线图步骤 23）：
    /// 卸载器/清单/快捷方式/注册表全部写入后执行
    pub exec_after: Vec<RunCommandSpec>,
    /// 卸载前执行命令（如停服务/关闭进程，路线图步骤 23）：
    /// 失败不中止卸载（占用文件走重试/重启删除路径），仅记录错误
    pub exec_before_uninstall: Vec<RunCommandSpec>,
    /// 环境变量写入（NSIS WriteEnvStr 惯例 / Inno ChangesEnvironment 对齐，路线图
    /// 步骤 27）：安装时写 HKCU/HKLM 环境区并广播 WM_SETTINGCHANGE，卸载时按
    /// 清单精确清理（新建值删除、PATH 段移除回写）
    pub env: Vec<EnvSpec>,
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
            uninstall_background: None,
            min_version: String::new(),
            allow_root_dir_install: false,
            exec_before: Vec::new(),
            exec_after: Vec::new(),
            exec_before_uninstall: Vec::new(),
            env: Vec::new(),
        }
    }
}

/// 单条环境变量写入（NSIS WriteEnvStr 惯例 / Inno ChangesEnvironment 对齐，路线图
/// 步骤 27）。安装时写入 Windows 环境区（用户 = HKCU\Environment，机器 =
/// HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment），
/// 写入后广播 WM_SETTINGCHANGE（新进程立即可见，无需注销）；卸载时按清单精确清理。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EnvSpec {
    /// 变量名（如 `PATH` / `JAVA_HOME`，不含 `=` 或 `%`）
    pub name: String,
    /// 值；支持 `{install_dir}` / `{app}` 模板。`part = "all"` 时整体设值；
    /// `path_append` / `path_prepend` 时必须是**单个路径段**（不能含 `;`）
    pub value: String,
    /// 写入方式：`all`（直接设值，默认）| `path_append`（追加到现有列表）|
    /// `path_prepend`（前置到现有列表）——后两者按 `;` 列表语义去重合并
    /// （NSIS WriteEnvStr / Inno PATH 编辑惯例）
    pub part: String,
    /// 作用域：`user`（HKCU，无需管理员，默认）| `machine`（HKLM，需管理员）
    pub scope: String,
}

impl Default for EnvSpec {
    fn default() -> Self {
        Self {
            name: String::new(),
            value: String::new(),
            part: "all".into(),
            scope: "user".into(),
        }
    }
}

/// 自定义命令（NSIS `Exec`/`ExecWait` 语义，路线图步骤 23）。
/// `{install_dir}` 模板在执行时替换为实际安装目录。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RunCommandSpec {
    /// 可执行文件：绝对路径 / 系统 PATH 内程序名；支持 `{install_dir}` 模板
    pub cmd: String,
    /// 参数（引号感知切分）；支持 `{install_dir}` 模板
    pub args: String,
    /// 工作目录；空 = 安装目录；支持 `{install_dir}` 模板
    pub working_dir: String,
    /// 退出码非 0 或启动失败时中止（默认 false：仅记录日志继续）
    pub abort_on_error: bool,
    /// 程序不存在（未随包安装/系统未提供）时忽略而不报错（默认 false）
    pub optional: bool,
}

impl Default for RunCommandSpec {
    fn default() -> Self {
        Self {
            cmd: String::new(),
            args: String::new(),
            working_dir: String::new(),
            abort_on_error: false,
            optional: false,
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
    /// 父分组路径（NSIS `SectionGroup`/`SectionGroupEnd` 嵌套等价，路线图步骤 24）：
    /// 如 `"Tools/Compression"`（`/` 分隔多级），空 = 根级。中间组自动创建。
    pub group: String,
}

impl Default for ComponentSpec {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            required: false,
            files: Vec::new(),
            group: String::new(),
        }
    }
}

/// 安装类型（NSIS `InstallTypes` / Inno `[Types]` 风格）：
/// 组件页顶部单选，选中时按 `components` 重置组件勾选状态；
/// 手动改动任意组件自动切到 Custom（NSIS 语义，见 `installtypes` 模块）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct InstallTypeSpec {
    /// 类型名称（如 Typical / Full / Minimal）
    pub name: String,
    /// 该类型默认勾选的组件索引集合（必装组件始终勾选）
    pub components: Vec<usize>,
    /// 是否为默认类型（未标记时取第一个）
    pub default: bool,
}

impl Default for InstallTypeSpec {
    fn default() -> Self {
        Self {
            name: String::new(),
            components: Vec::new(),
            default: false,
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
    /// 语言包定义（NSIS MUI2 `MUI_LANGUAGE` 对齐，路线图步骤 15）；
    /// 空 = 沿用英文默认文案（旧包行为不变）；多于一个时欢迎页前显示语言选择页
    pub languages: Vec<LanguageSpec>,
    /// 启动时按系统 UI 语言自动预选（默认 true）
    pub language_auto_detect: bool,
    /// UI 文案覆盖（键值见 README；语言无关的作者强制覆盖，优先级高于语言包）
    pub text: BTreeMap<String, String>,
    /// 内置界面模板 id（路线图步骤 21，五选一：aurora / midnight /
    /// sunrise / forest / mono，大小写不敏感）；空 = 不启用模板
    /// （沿用 accent 与内置默认色）。启用时整体接管调色板
    pub template: String,
    /// 是否显示左侧步骤导航栏（HM NIS Edit 向导式导航，路线图步骤 21；默认 true）
    pub show_step_rail: bool,
    /// 界面材质：`glass`（液态玻璃：半透明卡片 + 高光描边，默认）或
    /// `standard`（不透明卡片，旧观感）
    pub material: String,
    /// Windows 11 亚克力背景板（DWM SystemBackdrop，需 Win11 22H2+；
    /// 默认 false——失败平台自动回退为绘制的玻璃卡片，观感不受损）
    pub acrylic: bool,
    /// 背景渐变顶部色（十六进制；空 = 内置默认。模板启用时被模板覆盖）
    pub bg_top: String,
    /// 背景渐变底部色（同上）
    pub bg_bottom: String,
    /// 卡片底色（同上）
    pub card: String,
    /// 输入框/字段底色（同上）
    pub field: String,
    /// 卡片描边色（同上）
    pub card_stroke: String,
    /// 主文字色（同上）
    pub text_color: String,
    /// 次要文字色（同上）
    pub text_dim_color: String,
}

/// 语言包定义（NSIS `MUI_LANGUAGE` / Inno `[Languages]` 风格）。
/// `id` 与内置包（`miedit_core::langs`）匹配时缺键由内置包补齐；
/// `text` 中出现的键覆盖内置包。`ui.text` 仍是最高优先级覆盖。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LanguageSpec {
    /// 语言 id（BCP 47 风格，如 zh-CN / en-US；与内置包 id/别名匹配时自动补全缺键）
    pub id: String,
    /// 语言选择页显示名（如 "简体中文" / "English"）
    pub name: String,
    /// 该语言的文案覆盖键值表
    pub text: BTreeMap<String, String>,
}

impl Default for LanguageSpec {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            text: BTreeMap::new(),
        }
    }
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
            languages: Vec::new(),
            language_auto_detect: true,
            text: BTreeMap::new(),
            template: String::new(),
            show_step_rail: true,
            material: "glass".into(),
            acrylic: false,
            bg_top: String::new(),
            bg_bottom: String::new(),
            card: String::new(),
            field: String::new(),
            card_stroke: String::new(),
            text_color: String::new(),
            text_dim_color: String::new(),
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
    /// 快捷方式条目（Inno [Icons] 风格，路线图步骤 13）；
    /// 与上方简单开关叠加生效，旧脚本无此字段保持兼容
    pub entries: Vec<ShortcutEntry>,
}

impl Default for ShortcutsConfig {
    fn default() -> Self {
        Self {
            start_menu: true,
            desktop: false,
            name: None,
            run_args: String::new(),
            entries: Vec::new(),
        }
    }
}

/// 单条快捷方式（Inno `[Icons]` 条目风格）。
/// `locations` 取值：`start_menu` / `desktop` / `quick_launch`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ShortcutEntry {
    /// 快捷方式名称（不含 .lnk 后缀）
    pub name: String,
    /// 目标：包内相对路径；空 = 主程序 `app.exe`
    pub exe: String,
    /// 启动参数
    pub args: String,
    /// 开始菜单子目录（空 = 应用名分组目录，随卸载一并清除）
    pub subfolder: String,
    /// 图标：包内相对路径（安装后的文件）；空 = 目标自身图标
    pub icon: String,
    /// 工作目录；空 = 目标所在目录；支持 {install_dir} 模板
    pub working_dir: String,
    /// 快捷方式描述（鼠标悬停提示）
    pub comment: String,
    /// 创建位置列表（start_menu / desktop / quick_launch）
    pub locations: Vec<String>,
}

impl Default for ShortcutEntry {
    fn default() -> Self {
        Self {
            name: String::new(),
            exe: String::new(),
            args: String::new(),
            subfolder: String::new(),
            icon: String::new(),
            working_dir: String::new(),
            comment: String::new(),
            locations: vec!["start_menu".into()],
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
    /// 字符串值（REG_SZ）；`{install_dir}` 会在安装时替换
    pub values: BTreeMap<String, String>,
    /// 注册表视图（NSIS `SetRegView` 语义，路线图步骤 22）：
    /// `default`（默认）/ `64`（KEY_WOW64_64KEY）/ `32`（KEY_WOW64_32KEY），大小写不敏感
    pub view: String,
    /// DWORD 值（字符串形式：十进制或 `0x` 前缀十六进制，如 `"1"` / `"0x80"`；负数按 32 位回绕）
    pub dwords: BTreeMap<String, String>,
    /// 二进制值（REG_BINARY，十六进制字符串，可含空白，长度必须为偶数，如 `"DE AD BE EF"`）
    pub binary: BTreeMap<String, String>,
    /// REG_EXPAND_SZ 值（含 `%VAR%` 环境变量引用的字符串，安装时原样写入；`{install_dir}` 模板可用）
    pub expand_sz: BTreeMap<String, String>,
    /// REG_MULTI_SZ 值（多字符串列表）
    pub multi_sz: BTreeMap<String, Vec<String>>,
}

/// 读取 UI 文案：优先使用脚本覆盖，否则使用内置英文默认值。
/// （多语言请用 [`resolved_text`]；本函数等价于未选择语言时的行为，旧代码不变。）
pub fn ui_text(cfg: &Config, key: &str) -> String {
    resolved_text(cfg, None, key)
}

/// 按选中语言解析 UI 文案（路线图步骤 15，NSIS MUI 语义）。
///
/// 优先级（高 → 低）：
/// 1. `ui.text` 单键覆盖（作者强制，语言无关，兼容旧行为）
/// 2. 选中语言包的 `text` 表（`cfg.ui.languages[lang]`）
/// 3. 内置语言包（`miedit_core::langs`，按语言 id/别名匹配）
/// 4. 英文默认表 [`default_text`]
///
/// `lang` 越界/为 None 时跳过 2、3（3 仅在语言 id 能匹配内置包时生效）。
pub fn resolved_text(cfg: &Config, lang: Option<usize>, key: &str) -> String {
    if let Some(v) = cfg.ui.text.get(key) {
        return v.clone();
    }
    resolve_lang_text(&cfg.ui.languages, lang, key)
}

/// 语言包维度的文案解析（`resolved_text` 的底层链，供安装器与卸载器共用）。
/// `ui.text` 作者强制覆盖之外的四段链：语言包 text > 内置包 > 英文默认。
pub fn resolve_lang_text(
    languages: &[LanguageSpec],
    lang: Option<usize>,
    key: &str,
) -> String {
    if let Some(i) = lang {
        if let Some(spec) = languages.get(i) {
            if let Some(v) = spec.text.get(key) {
                return v.clone();
            }
            if let Some(builtin) = crate::langs::builtin_language_for_id(&spec.id) {
                if let Some((_, v)) = builtin.texts.iter().find(|(k, _)| *k == key) {
                    return (*v).to_string();
                }
            }
        }
    }
    match default_text(key) {
        Some(s) => s.to_string(),
        None => key.to_string(),
    }
}

/// 卸载器文案解析（路线图步骤 26，NSIS un MUI 对齐）。
///
/// - 旧清单（无语言包，`languages` 为空）：回退内置 zh-CN 包——历史卸载器
///   文案为中文硬编码，保持旧包行为完全不变；
/// - 新清单（有语言包）：与安装器 [`resolve_lang_text`] 同链——语言包 text >
///   按语言 id 匹配的内置包 > 英文默认表（卸载器语言跟随安装语言，NSIS 语义）。
pub fn resolve_uninstall_text(
    languages: &[LanguageSpec],
    lang_idx: Option<usize>,
    key: &str,
) -> String {
    if languages.is_empty() {
        if let Some((_, v)) = crate::langs::ZH_CN.texts.iter().find(|(k, _)| *k == key) {
            return (*v).to_string();
        }
    }
    resolve_lang_text(languages, lang_idx, key)
}

/// 卸载器专用文案键（步骤 26；`default_text` 与内置 zh-CN 包均含全部键，
/// 键集一致性测试守护）。卸载器据此构建全量文案快照。
pub static UNINSTALL_TEXT_KEYS: &[&str] = &[
    "un_window_title",
    "un_confirm_title",
    "un_confirm_body",
    "un_running_warning",
    "un_recheck",
    "un_delete_user_data",
    "un_btn_uninstall",
    "un_btn_next",
    "un_btn_cancel",
    "un_btn_finish",
    "un_btn_close",
    "un_working_title",
    "un_progress_items",
    "un_summary_title",
    "un_stats",
    "un_reboot_note",
    "un_done_body",
    "un_done_reboot_body",
    "un_failed_title",
    "un_language_title",
    "un_language_desc",
    "un_error_admin",
    "un_log_start",
    "un_log_running_warning",
    "un_log_reboot_pending",
    "un_log_cmd_label",
    "un_log_exec",
    "un_log_cmd_failed",
    "un_log_reboot_scheduled",
    "un_log_delete",
    "un_log_reboot_delete",
    "un_log_remove_dir",
    "un_log_dir_kept",
    "un_log_del_file_failed",
    "un_log_del_shortcut_failed",
    "un_log_del_shortcut_dir_failed",
    "un_log_del_user_data",
    "un_log_reg_value",
    "un_log_reg_key",
    "un_log_uninstall_reg",
    "un_log_env",
    "un_log_env_path",
    "un_log_done",
    "un_log_done_errors",
    "un_silent_stderr",
];

/// 校验卸载器文案键集完整性：UNINSTALL_TEXT_KEYS 必须被英文默认表
/// 与内置 zh-CN 包全部覆盖（编译期防漂移，运行期不调用）。
#[doc(hidden)]
pub fn uninstall_text_keys_covered() -> bool {
    UNINSTALL_TEXT_KEYS.iter().all(|k| {
        default_text(k).is_some()
            && crate::langs::ZH_CN.texts.iter().any(|(zk, _)| *zk == *k)
    })
}

/// 内置默认 UI 文案表（英文基调）；未知键返回 None。
/// 公开供 `langs` 模块做键集一致性检查。
pub fn default_text(key: &str) -> Option<&'static str> {
    DEFAULT_TEXT_KEYS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| *v)
}

/// 内置英文默认文案表（`pub` 供键集一致性测试）。
pub static DEFAULT_TEXT_KEYS: &[(&str, &str)] = &[
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
        // 步骤 10：InstallTypes 类型单选行
        ("components_type_title", "Type of install:"),
        ("components_type_custom", "Custom"),
        // 步骤 11：组件描述面板
        ("components_desc_panel", "Component description"),
        ("components_desc_empty", "Hover or click a component to see its description."),
        ("free_space", "Free space on drive: {free}"),
        ("app_running_title", "App Is Running"),
        ("app_running_body", "{app} appears to be running. Please close it before continuing, or retry the check."),
        ("retry", "Retry"),
        ("continue_anyway", "Continue Anyway"),
        ("upgrade_notice", "Found installed version {old}; it will be upgraded to {new}."),
        // 步骤 8：文件写入错误模态（NSIS 文件错误对话框：重试/忽略/中止）
        //（英文基调修正：原中文值移入内置 zh-CN 语言包）
        ("file_error_title", "Failed to Write File"),
        ("file_error_body", "Unable to write the following file:\n{file}\n\n{error}"),
        (
            "file_error_hint",
            "Close programs using this file and choose Retry; Ignore skips it and replaces it after reboot; Abort stops the installation (files already written are kept).",
        ),
        ("file_error_ignore", "Ignore"),
        ("file_error_abort", "Abort"),
        // 步骤 9：完成页"需要重启"琥珀提示
        (
            "finish_reboot_required",
            "A restart is required to finish installation: {count} file(s) will be replaced after reboot.",
        ),
        // 步骤 15：多语言框架新增键 + 原硬编码 UI 字符串文案化
        ("language_title", "Select Language"),
        ("language_desc", "Please select the language used by this installer."),
        ("version_line", "Version {version} · {publisher}"),
        (
            "license_scroll_hint",
            "Please scroll the license agreement to the bottom first.",
        ),
        (
            "dir_exists_warning",
            "Note: the target folder already exists and is not empty. Files inside will be overwritten or merged.",
        ),
        ("progress_stats", "Extracted {done} / {total} files · {bytes} · {speed}"),
        ("details_show", "Show details"),
        ("details_hide", "Hide details"),
        ("finish_reboot_more", "and {count} files in total (see install.log)"),
        // 步骤 17：ARP 修复入口 / 修复模式确认页与完成页
        ("repair_title", "Repair Installation"),
        (
            "repair_body",
            "Setup is ready to repair {app} {version}.\nAll program files will be re-extracted, and shortcuts and registry entries will be rebuilt.\nInstall folder: {dir}",
        ),
        (
            "repair_summary",
            "File rules: {files} · Shortcuts: {shortcuts} · Registry values: {registry}",
        ),
        ("repair_now", "Repair"),
        ("repair_finish_title", "Repair Complete"),
        ("repair_finish_body", "{app} {version} has been repaired at:"),
        // 步骤 20：目录页校验提示（NSIS DirectoryPage：无效路径/根目录拒绝）
        (
            "dir_invalid",
            "Please enter a valid absolute install folder on an existing drive.",
        ),
        (
            "dir_root_warning",
            "For safety reasons, installing into a drive root is not allowed. Please choose a subfolder.",
        ),
        // 步骤 21：左侧步骤导航栏（HM NIS Edit 向导式导航）
        ("nav_welcome", "Welcome"),
        ("nav_license", "License"),
        ("nav_components", "Components"),
        ("nav_dir", "Install Location"),
        ("nav_install", "Install"),
        ("nav_finish", "Finish"),
        // 步骤 26：卸载器多语言（NSIS un MUI 对齐）——卸载器全部 UI 与日志文案。
        // 占位符约定：{app} 应用名 / {version} 版本 / {dir} 目录 / {exe} 主程序
        // / {file} 文件 / {count} 数量 / {files} {dirs} {size} 卸载统计 / {done} {total}
        // 进度 / {label} 命令标签 / {reason} 原因 / {err} {rerr} 错误 / {path} 路径
        // / {name} 变量名 / {segment} PATH 段
        ("un_window_title", "Uninstall {app}"),
        ("un_confirm_title", "Uninstall {app}"),
        (
            "un_confirm_body",
            "This will remove {app} {version} and all of its components from your computer.\nInstall folder: {dir}",
        ),
        (
            "un_running_warning",
            "{app} appears to be running. It is recommended to close it before uninstalling.\nFiles in use will be deleted after a reboot.",
        ),
        ("un_recheck", "Re-check"),
        ("un_delete_user_data", "Also delete user data and settings"),
        ("un_btn_uninstall", "Uninstall"),
        ("un_btn_next", "Next"),
        ("un_btn_cancel", "Cancel"),
        ("un_btn_finish", "Finish"),
        ("un_btn_close", "Close"),
        ("un_working_title", "Uninstalling {app}"),
        ("un_progress_items", "{done} / {total} items"),
        ("un_summary_title", "{app} was successfully uninstalled"),
        (
            "un_stats",
            "Removed {files} file(s) and {dirs} folder(s), freed {size}",
        ),
        ("un_reboot_note", "{count} file(s) will be deleted after the system restarts."),
        (
            "un_done_body",
            "After closing this window, the uninstaller will clean up its own files.",
        ),
        (
            "un_done_reboot_body",
            "{count} file(s) were in use and are scheduled for deletion after reboot; the cleanup will finish then.",
        ),
        ("un_failed_title", "Uninstall Problem"),
        ("un_language_title", "Select Uninstall Language"),
        ("un_language_desc", "Please select the language used by this uninstaller."),
        ("un_error_admin", "Administrator privileges are required to uninstall this installation."),
        ("un_log_start", "Starting uninstall of {app}"),
        (
            "un_log_running_warning",
            "Warning: main program {exe} may be running; locked files will be deleted after reboot",
        ),
        (
            "un_log_reboot_pending",
            "Note: {count} file(s) will be deleted after the system restarts",
        ),
        ("un_log_cmd_label", "pre-uninstall command #{n}"),
        ("un_log_exec", "Running {label}: {cmd}"),
        (
            "un_log_cmd_failed",
            "{label} failed: {reason} (ignored, continuing uninstall)",
        ),
        (
            "un_log_reboot_scheduled",
            "Scheduled for deletion after reboot (file in use): {file}",
        ),
        ("un_log_delete", "Deleting {file}"),
        ("un_log_reboot_delete", "Reboot-delete {file}"),
        ("un_log_remove_dir", "Removing folder {dir}"),
        ("un_log_dir_kept", "Folder not empty, kept: {dir}"),
        ("un_log_del_file_failed", "Delete failed: {err}; scheduling reboot deletion also failed: {rerr}"),
        ("un_log_del_shortcut_failed", "Failed to delete shortcut: {path}"),
        ("un_log_del_shortcut_dir_failed", "Failed to delete shortcut folder: {path}"),
        ("un_log_del_user_data", "Deleting user data {path}"),
        ("un_log_reg_value", "Removing registry value {key}"),
        ("un_log_reg_key", "Removing registry key {key}"),
        ("un_log_uninstall_reg", "Removing uninstall registry information"),
        ("un_log_env", "Removing environment variable {name}"),
        ("un_log_env_path", "Removing {segment} from {name}"),
        ("un_log_done", "Uninstall complete"),
        (
            "un_log_done_errors",
            "Uninstall finished with {count} item(s) that could not be removed",
        ),
        (
            "un_silent_stderr",
            "Uninstall finished with {count} error(s) (see log)",
        ),
    ];

/// 将文案中的 `{app}` / `{version}` 替换为实际值。
pub fn fill_template(s: &str, app: &str, version: &str) -> String {
    s.replace("{app}", app).replace("{version}", version)
}

// ---------------------------------------------------------------------------
// 测试（步骤 26/27：卸载器多语言键集、解析链、清单新旧兼容）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod step_tests {
    use super::*;
    use crate::manifest::{EnvCleanupRef, UninstallManifest};

    /// 卸载器专用键必须被英文默认表与内置 zh-CN 包同时覆盖（防漂移）。
    #[test]
    fn uninstall_text_keys_are_covered() {
        assert!(uninstall_text_keys_covered());
        // UNINSTALL_TEXT_KEYS 无重复
        let mut keys: Vec<&str> = UNINSTALL_TEXT_KEYS.to_vec();
        keys.sort();
        let n = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), n, "UNINSTALL_TEXT_KEYS 存在重复键");
    }

    /// 旧清单（无语言包）→ 卸载器回退内置 zh-CN（历史中文文案行为不变）。
    #[test]
    fn resolve_uninstall_text_falls_back_to_zh() {
        let t = resolve_uninstall_text(&[], None, "un_confirm_title");
        assert_eq!(t, "卸载 {app}");
        // 未知键回退键名
        assert_eq!(resolve_uninstall_text(&[], None, "nonexistent_key"), "nonexistent_key");
    }

    /// 解析链优先级：语言包 text > 按语言 id 匹配内置包 > 英文默认表；
    /// zh-CN 语言包命中内置中文包。
    #[test]
    fn resolve_uninstall_text_chain() {
        let mut pack = LanguageSpec::default();
        pack.id = "en-US".into();
        pack.name = "English".into();
        pack.text.insert("un_confirm_title".into(), "Custom Title".into());
        let mut zh = LanguageSpec::default();
        zh.id = "zh-CN".into();
        zh.name = "简体中文".into();
        let langs = vec![pack.clone(), zh];
        // 0. 语言包覆盖
        assert_eq!(resolve_uninstall_text(&langs, Some(0), "un_confirm_title"), "Custom Title");
        // 1. zh-CN 包按 id 命中内置中文包
        assert_eq!(resolve_uninstall_text(&langs, Some(1), "un_confirm_title"), "卸载 {app}");
        // 2. en-US 无覆盖 → 内置 en-US 空表 → 英文默认表
        assert_eq!(resolve_uninstall_text(&langs, Some(0), "un_btn_next"), "Next");
        // 3. 越界索引 → 回退英文默认表
        assert_eq!(resolve_uninstall_text(&langs, Some(9), "un_btn_next"), "Next");
    }

    /// 清单新字段（languages/language/env_values）往返；旧 JSON（无新字段）→ serde default 兼容。
    #[test]
    fn manifest_new_fields_roundtrip_and_old_compat() {
        let m = UninstallManifest {
            app_name: "App".into(),
            languages: vec![LanguageSpec::default()],
            language: "zh-CN".into(),
            env_values: vec![EnvCleanupRef {
                name: "Path".into(),
                machine: false,
                path_segment: "C:\\Apps\\App".into(),
                existed: true,
                previous_value: String::new(),
            }],
            ..Default::default()
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: UninstallManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.languages.len(), 1);
        assert_eq!(back.language, "zh-CN");
        assert_eq!(back.env_values.len(), 1);
        assert_eq!(back.env_values[0].path_segment, "C:\\Apps\\App");
        // 旧清单 JSON（无 languages/language/env_values 字段）
        let old = r#"{"app_name":"Old","files":[]}"#;
        let old_m: UninstallManifest = serde_json::from_str(old).unwrap();
        assert!(old_m.languages.is_empty());
        assert!(old_m.language.is_empty());
        assert!(old_m.env_values.is_empty());
    }

    /// EnvSpec serde default 兼容（旧脚本无 env 字段 / 缺省 part/scope）。
    #[test]
    fn env_spec_defaults() {
        let e: EnvSpec = serde_json::from_str(r#"{"name":"X","value":"1"}"#).unwrap();
        assert_eq!(e.part, "all");
        assert_eq!(e.scope, "user");
        let cfg_json = r#"{"app":{"name":"A"},"install":{}}"#;
        let c: Config = serde_json::from_str(cfg_json).unwrap();
        assert!(c.install.env.is_empty());
    }
}
