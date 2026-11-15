//! MIEdit (Ming-Install-Edit) 核心库。
//!
//! 定义安装包格式（归档 + 包尾）、安装脚本配置、卸载清单，
//! 以及安装器/卸载器共用的 Windows 工具函数。

pub mod archive;
pub mod config;
pub mod crc32;
pub mod fileerror;
pub mod files;
pub mod installtypes;
pub mod langs;
pub mod manifest;
pub mod overwrite;
pub mod repair;
pub mod sysreq;
pub mod trailer;
pub mod util;
pub mod versioninfo;
pub mod winutil;

/// 当前 MIEdit 版本
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
