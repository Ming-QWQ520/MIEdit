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
}

/// 单条自定义注册表值的清理引用（步骤 22）。
#[derive(Debug, Clone, Serialize, Deserialize)]
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
