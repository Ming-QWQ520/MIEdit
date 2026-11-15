//! 读取自身 EXE 尾部的包数据（配置 JSON、背景图、图标）。
//!
//! 启动时默认流式校验 payload/config/uninstaller 三个分区的 CRC32
//! （NSIS `CRCCheck` 对齐，MIEDPKG3 包尾）；`--ncrc` 跳过（CRCCheck off 语义）；
//! 旧格式包（MIEDPKG2，无 CRC）直接放行，保持向后兼容。

use anyhow::{anyhow, Context, Result};
use miedit_core::config::Config;
use miedit_core::trailer::{self, Range, Trailer};
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

/// 加载完成的安装包（不含文件句柄；提取时按需重新打开）。
pub struct Package {
    pub path: PathBuf,
    pub config: Config,
    pub trailer: Trailer,
    /// 归档中所有文件未压缩总字节数
    pub total_bytes: u64,
    /// 全窗口背景图（egui 可直接上传纹理）
    pub bg: Option<egui::ColorImage>,
    /// 应用图标（用于窗口图标）
    pub icon: Option<egui::IconData>,
}

use eframe::egui;

fn read_range(f: &mut File, r: Range) -> Result<Vec<u8>> {
    trailer::read_range(f, r)
}

/// 解析归档头，拿到未压缩总字节数。
fn read_total_bytes(f: &mut File, payload: Range) -> Result<u64> {
    use std::io::{Read, Seek, SeekFrom};
    f.seek(SeekFrom::Start(payload.off))?;
    let mut head = [0u8; 16];
    f.read_exact(&mut head)?;
    if &head[0..8] != miedit_core::archive::ARCHIVE_MAGIC {
        return Err(anyhow!("payload 归档头无效"));
    }
    Ok(u64::from_le_bytes(head[8..16].try_into().unwrap()))
}

/// 加载当前 EXE 中的安装包数据。
/// `ncrc = true` 时跳过 CRC32 校验（NSIS `/NCRC` 语义）。
pub fn load_self_package(ncrc: bool) -> Result<Arc<Package>> {
    let path = std::env::current_exe().context("获取自身路径失败")?;
    let tr = trailer::read_trailer(&path)?.ok_or_else(|| {
        anyhow!("这不是有效的 MIEdit 安装包（未找到包尾标记）。请使用 miedit-compiler 生成安装包。")
    })?;

    // NSIS CRCCheck 对齐：默认校验关键分区 CRC32，损坏包拒绝安装
    if !ncrc {
        trailer::verify_crc(&path, &tr)?;
    }

    let mut f = File::open(&path).context("打开自身失败")?;

    let cfg_bytes = read_range(&mut f, tr.config)?;
    let config: Config =
        serde_json::from_slice(&cfg_bytes).context("解析安装包配置失败")?;

    let total_bytes = read_total_bytes(&mut f, tr.payload).unwrap_or(0);

    let bg = if !tr.background.is_empty() {
        let bytes = read_range(&mut f, tr.background)?;
        decode_image_color(&bytes).ok()
    } else {
        None
    };
    let icon = if !tr.icon.is_empty() {
        let bytes = read_range(&mut f, tr.icon)?;
        decode_image_icon(&bytes).ok()
    } else {
        None
    };

    Ok(Arc::new(Package {
        path,
        config,
        trailer: tr,
        total_bytes,
        bg,
        icon,
    }))
}

/// 解码为 egui ColorImage。
fn decode_image_color(bytes: &[u8]) -> Result<egui::ColorImage> {
    let img = image::load_from_memory(bytes).context("背景图解码失败")?;
    let rgba = img.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

/// 解码为窗口图标 IconData。
fn decode_image_icon(bytes: &[u8]) -> Result<egui::IconData> {
    let img = image::load_from_memory(bytes).context("图标解码失败")?;
    let rgba = img.to_rgba8();
    Ok(egui::IconData {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.as_raw().clone(),
    })
}
