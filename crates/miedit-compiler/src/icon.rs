//! PE 资源替换（Windows）：通过 `UpdateResourceW` 在**同一事务**中写入
//! 图标资源（RT_ICON / RT_GROUP_ICON）与版本信息资源（RT_VERSION，
//! VS_VERSIONINFO 字节序列由 `miedit_core::versioninfo` 构造）。
//! 对齐 NSIS 的 SetIcon + `VIProductVersion` / `VIAddVersionKey` 能力。

use anyhow::{bail, Result};
use std::path::Path;

/// 一次事务写入 PE 资源：`ico_bytes` 替换图标，`versioninfo` 写入 RT_VERSION #1。
/// 两者都为 None 时不做任何事；任一写入失败则整个事务回滚（EndUpdateResource 不提交）。
#[cfg(windows)]
pub fn patch_pe_resources(
    pe: &Path,
    ico_bytes: Option<&[u8]>,
    versioninfo: Option<&[u8]>,
) -> Result<()> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::BOOL;
    use windows::Win32::System::LibraryLoader::{
        BeginUpdateResourceW, EndUpdateResourceW, UpdateResourceW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{RT_GROUP_ICON, RT_ICON, RT_VERSION};

    /// MAKEINTRESOURCEW 未被 windows-rs 绑定：其结果就是把整数值当作指针。
    fn make_int_res(id: u16) -> PCWSTR {
        PCWSTR(id as usize as *const u16)
    }

    if ico_bytes.is_none() && versioninfo.is_none() {
        return Ok(());
    }

    let (group, images) = match ico_bytes {
        Some(b) => {
            let (g, i) = parse_ico(b)?;
            (Some(g), Some(i))
        }
        None => (None, None),
    };

    let pe_h = HSTRING::from(pe.as_os_str());
    let handle = unsafe {
        BeginUpdateResourceW(PCWSTR(pe_h.as_ptr()), false).map_err(|e| {
            anyhow::anyhow!("BeginUpdateResourceW 失败: {e}（文件可能被占用）")
        })?
    };

    let mut failed = None;
    unsafe {
        if let Some(images) = &images {
            for (i, img) in images.iter().enumerate() {
                if let Err(e) = UpdateResourceW(
                    handle,
                    RT_ICON,
                    make_int_res((i + 1) as u16),
                    0,
                    Some(img.as_ptr().cast::<core::ffi::c_void>()),
                    img.len() as u32,
                ) {
                    failed = Some(format!("写入 RT_ICON #{} 失败: {e}", i + 1));
                    break;
                }
            }
        }
        if failed.is_none() {
            if let Some(group) = &group {
                if let Err(e) = UpdateResourceW(
                    handle,
                    RT_GROUP_ICON,
                    make_int_res(1),
                    0,
                    Some(group.as_ptr().cast::<core::ffi::c_void>()),
                    group.len() as u32,
                ) {
                    failed = Some(format!("写入 RT_GROUP_ICON 失败: {e}"));
                }
            }
        }
        // 版本信息资源（同一事务；先图标后版本，避免资源表重复打开）
        if failed.is_none() {
            if let Some(vi) = versioninfo {
                if let Err(e) = UpdateResourceW(
                    handle,
                    RT_VERSION,
                    make_int_res(1),
                    0,
                    Some(vi.as_ptr().cast::<core::ffi::c_void>()),
                    vi.len() as u32,
                ) {
                    failed = Some(format!("写入 RT_VERSION 失败: {e}"));
                }
            }
        }
        if failed.is_none() {
            if let Err(e) = EndUpdateResourceW(handle, BOOL::from(false)) {
                failed = Some(format!("EndUpdateResourceW 失败: {e}"));
            }
        }
    }
    if let Some(msg) = failed {
        bail!(msg);
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn patch_pe_resources(
    _pe: &Path,
    _ico_bytes: Option<&[u8]>,
    _versioninfo: Option<&[u8]>,
) -> Result<()> {
    bail!("PE 资源替换（图标/版本信息）仅支持 Windows")
}

/// 解析 .ico：返回（RT_GROUP_ICON 数据, 各 RT_ICON 图像数据）。
fn parse_ico(data: &[u8]) -> Result<(Vec<u8>, Vec<Vec<u8>>)> {
    if data.len() < 6 {
        bail!("ICO 文件过小");
    }
    let typ = u16::from_le_bytes([data[2], data[3]]);
    if typ != 1 {
        bail!("不是有效的 ICO 文件（type={typ}）");
    }
    let count = u16::from_le_bytes([data[4], data[5]]) as usize;
    if count == 0 {
        bail!("ICO 不包含任何图像");
    }
    let mut images = Vec::with_capacity(count);
    let mut entries: Vec<(u8, u8, u8, u16, u16, usize)> = Vec::with_capacity(count);
    for i in 0..count {
        let base = 6 + i * 16;
        if base + 16 > data.len() {
            bail!("ICO 目录项越界");
        }
        let w = data[base];
        let h = data[base + 1];
        let colors = data[base + 2];
        let planes = u16::from_le_bytes([data[base + 4], data[base + 5]]);
        let bpp = u16::from_le_bytes([data[base + 6], data[base + 7]]);
        let size = u32::from_le_bytes(data[base + 8..base + 12].try_into().unwrap()) as usize;
        let off = u32::from_le_bytes(data[base + 12..base + 16].try_into().unwrap()) as usize;
        if off + size > data.len() {
            bail!("ICO 图像数据越界（#{}）", i + 1);
        }
        images.push(data[off..off + size].to_vec());
        entries.push((w, h, colors, planes, bpp, size));
    }

    // GRPICONDIR + GRPICONDIRENTRY[]
    let mut group = Vec::with_capacity(6 + count * 14);
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&(count as u16).to_le_bytes());
    for (i, e) in entries.iter().enumerate() {
        let (w, h, colors, planes, bpp, size) = *e;
        group.push(w);
        group.push(h);
        group.push(colors);
        group.push(0);
        group.extend_from_slice(&planes.to_le_bytes());
        group.extend_from_slice(&bpp.to_le_bytes());
        group.extend_from_slice(&(size as u32).to_le_bytes());
        group.extend_from_slice(&((i + 1) as u16).to_le_bytes());
    }
    Ok((group, images))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_ico() {
        // 构造一个含单张 16x16 假图像的 ico
        let mut ico = vec![0u16.to_le_bytes(), 1u16.to_le_bytes(), 1u16.to_le_bytes()].concat();
        let img = vec![0xABu8; 32];
        let mut dir = vec![16u8, 16, 0, 0];
        dir.extend_from_slice(&1u16.to_le_bytes()); // planes
        dir.extend_from_slice(&32u16.to_le_bytes()); // bpp
        dir.extend_from_slice(&(img.len() as u32).to_le_bytes());
        dir.extend_from_slice(&22u32.to_le_bytes()); // offset = 6+16
        ico.extend_from_slice(&dir);
        ico.extend_from_slice(&img);
        let (group, images) = parse_ico(&ico).unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].len(), 32);
        assert_eq!(group.len(), 6 + 14);
        // GRPICONDIRENTRY 从偏移 6 开始：w,h,colors,reserved,planes,bpp,size,id
        assert_eq!(&group[6..10], &[16u8, 16, 0, 0]);
        let planes = u16::from_le_bytes(group[10..12].try_into().unwrap());
        assert_eq!(planes, 1);
        let bpp = u16::from_le_bytes(group[12..14].try_into().unwrap());
        assert_eq!(bpp, 32);
        let sz = u32::from_le_bytes(group[14..18].try_into().unwrap());
        assert_eq!(sz, 32);
        let id = u16::from_le_bytes(group[18..20].try_into().unwrap());
        assert_eq!(id, 1);
    }
}
