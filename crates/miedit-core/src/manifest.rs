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
    /// 额外写入的注册表键（含 hive 前缀，整树删除）
    pub registry_keys: Vec<String>,
    /// 卸载注册表键（不含 hive 前缀，如 SOFTWARE\...\Uninstall\{app_id}）
    pub uninstall_key: String,
    /// 用户数据/配置目录（安装时解析；仅当卸载时勾选“删除用户数据”才删除）
    pub user_data_dirs: Vec<String>,
    pub uninstaller_path: String,
    pub manifest_path: String,
    /// 安装时间（Unix 时间戳秒）
    pub installed_at: u64,
}
