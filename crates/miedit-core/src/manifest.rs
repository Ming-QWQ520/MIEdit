//! 卸载清单：安装器写入安装目录（uninstall.json），卸载器据此清理。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UninstallManifest {
    pub app_name: String,
    pub app_id: String,
    pub version: String,
    pub publisher: String,
    pub install_dir: String,
    /// 主程序相对路径（相对 install_dir；卸载前运行检测用，可为空）
    pub main_exe: String,
    /// true = 安装时写入了 HKLM（卸载时需要管理员）
    pub per_machine: bool,
    /// 已安装文件（绝对路径）
    pub files: Vec<String>,
    /// 已创建目录（相对安装目录，'/' 分隔）
    pub dirs: Vec<String>,
    /// 快捷方式 .lnk（绝对路径）
    pub shortcuts: Vec<String>,
    /// 快捷方式目录（绝对路径，如公共开始菜单的应用子目录）
    pub shortcut_dirs: Vec<String>,
    /// 额外写入的注册表键（含 hive 前缀，整树删除；步骤 22 起兼容保留：
    /// 新清单优先用 registry_values 精确清理，本字段仅用于计数/旧行为回退）
    pub registry_keys: Vec<String>,
    /// 自定义注册表值清理引用（步骤 22 精确清理）：新建键整树删除，
    /// 既有键只删本次写入的值（不误伤安装前已存在的内容）。
    /// 旧清单无此字段 → 卸载器回退整树删除 registry_keys
    pub registry_values: Vec<RegistryValueRef>,
    /// 卸载注册表键（不含 hive 前缀，如 SOFTWARE\...\Uninstall\{app_id}）
    pub uninstall_key: String,
    /// 用户数据/配置目录（安装时解析；仅当卸载时勾选“删除用户数据”才删除）
    pub user_data_dirs: Vec<String>,
    pub uninstaller_path: String,
    pub manifest_path: String,
    /// 安装时间（Unix 时间戳秒）
    pub installed_at: u64,
    /// 卸载前执行命令（步骤 23；安装器从脚本复制，卸载器删除阶段前执行）
    pub exec_before_uninstall: Vec<crate::config::RunCommandSpec>,
    /// 语言包副本（步骤 26，NSIS un MUI 对齐）：安装器把 `ui.languages`
    /// 随清单内嵌/落盘；多于一个时卸载器先显示语言选择页（单语言直接
    /// 使用；旧清单无此字段 → 空 → 回退内置 zh-CN，行为不变）
    pub languages: Vec<crate::config::LanguageSpec>,
    /// 安装时用户实际选定的语言 id（步骤 28，NSIS MUI 语义：安装语言页
    /// 的选择经注册表/清单持久化，卸载器**直接跟随**，不再自行询问——
    /// NSIS 官方模板 `un.onInit` 读回安装语言的惯例）。
    /// 空 = 旧清单或无语言包 → 卸载器回退自动预选（多语言旧清单仍显示
    /// 语言选择页，单语言/空清单行为不变）。
    pub language: String,
    /// 环境变量清理引用（步骤 27）：`created_value` = 安装时新建的值
    /// （卸载时直接删除）；否则若 `path_segment` 非空则从列表中移除该段后回写
    pub env_values: Vec<EnvCleanupRef>,
    /// 安装目录**之外**创建的目录（MI 脚本 v2 组件指令 `create_dir`，
    /// 路线图步骤 33；绝对路径）。卸载时按 NSIS `RMDir` 语义处理：
    /// 仅删除空目录，非空保留并记日志。旧清单无此字段 → 空 → 行为不变。
    pub extra_dirs: Vec<String>,
}

/// 单条自定义注册表值的清理引用（步骤 22）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RegistryValueRef {
    /// 含 hive 前缀的键（与 registry_keys 一致）
    pub key: String,
    /// 值名；空 = 仅键级清理（该 spec 无值写入时）
    pub name: String,
    /// 安装前该键不存在（卸载时可整树删除，避免误删既有内容）
    pub created_key: bool,
}

impl Default for RegistryValueRef {
    fn default() -> Self {
        Self {
            key: String::new(),
            name: String::new(),
            created_key: false,
        }
    }
}

/// 单条环境变量的清理引用（步骤 27，NSIS WriteEnvStr 惯例 / Inno
/// ChangesEnvironment 对齐）。按写入方式二分：
/// - `path_segment` 非空：PATH 式合并写入 → 卸载时从 `;` 列表中**移除该段**后
///   回写（ASCII 不区分大小写，[`crate::envspec`]；只删自己的段，不误伤其他
///   软件后来写入的段）；移除后为空且安装前不存在 → 删除整个值；
/// - `path_segment` 为空（`all` 直接设值）：`existed = false`（安装前变量不存在，
///   本次新建）→ 卸载时删除整个值；`existed = true` → 卸载时回写
///   `previous_value`（还原安装前的原值）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EnvCleanupRef {
    /// 变量名
    pub name: String,
    /// true = 机器级（HKLM 环境区，卸载需管理员）；false = 用户级（HKCU）
    pub machine: bool,
    /// PATH 式合并写入时记录的展开后路径段（`all` 写入为空）
    pub path_segment: String,
    /// `all` 写入时：安装前该变量是否已存在
    pub existed: bool,
    /// `all` 写入时安装前的原值（卸载时还原；`existed = false` 时为空并删除整个值）
    pub previous_value: String,
}

impl Default for EnvCleanupRef {
    fn default() -> Self {
        Self {
            name: String::new(),
            machine: false,
            path_segment: String::new(),
            existed: false,
            previous_value: String::new(),
        }
    }
}
