//! 安装包尾（Trailer）。
//!
//! 成品安装包 EXE 布局：
//! ```text
//! [stub EXE] [payload 归档] [config JSON] [卸载器 EXE] [背景图] [图标] [Trailer 88B]
//! ```
//! Trailer 位于文件最末尾 88 字节：
//! magic(8) + 5 个 (off:u64, len:u64)。

use anyhow::{bail, Context, Result};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::fs;
use std::path::Path;

pub const TRAILER_MAGIC: &[u8; 8] = b"MIEDPKG2";
pub const TRAILER_SIZE: u64 = 104;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Range {
    pub off: u64,
    pub len: u64,
}

impl Range {
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn end(&self) -> u64 {
        self.off + self.len
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Trailer {
    pub payload: Range,
    pub config: Range,
    pub uninstaller: Range,
    pub background: Range,
    pub icon: Range,
    /// 卸载器专用图标（可选；缺省复用 icon）
    pub uninstall_icon: Range,
}

impl Trailer {
    pub fn encode(&self) -> [u8; TRAILER_SIZE as usize] {
        let mut out = [0u8; TRAILER_SIZE as usize];
        out[0..8].copy_from_slice(TRAILER_MAGIC);
        let ranges = [
            self.payload,
            self.config,
            self.uninstaller,
            self.background,
            self.icon,
            self.uninstall_icon,
        ];
        let mut pos = 8;
        for r in ranges {
            out[pos..pos + 8].copy_from_slice(&r.off.to_le_bytes());
            out[pos + 8..pos + 16].copy_from_slice(&r.len.to_le_bytes());
            pos += 16;
        }
        out
    }

    pub fn decode(b: &[u8]) -> Option<Trailer> {
        if b.len() < TRAILER_SIZE as usize || &b[0..8] != TRAILER_MAGIC {
            return None;
        }
        let mut pos = 8;
        let mut next = || {
            let off = u64::from_le_bytes(b[pos..pos + 8].try_into().unwrap());
            let len = u64::from_le_bytes(b[pos + 8..pos + 16].try_into().unwrap());
            pos += 16;
            Range { off, len }
        };
        Some(Trailer {
            payload: next(),
            config: next(),
            uninstaller: next(),
            background: next(),
            icon: next(),
            uninstall_icon: next(),
        })
    }

    /// 校验所有分区都落在文件内。
    fn sane(&self, file_len: u64) -> bool {
        let limit = file_len.saturating_sub(TRAILER_SIZE);
        [
            self.payload,
            self.config,
            self.uninstaller,
            self.background,
            self.icon,
            self.uninstall_icon,
        ]
        .iter()
            .all(|r| r.end() <= limit && (r.len == 0 || r.off < limit))
    }
}

/// 读取文件末尾的 Trailer；没有则返回 None。
pub fn read_trailer(path: &Path) -> Result<Option<Trailer>> {
    let mut f = File::open(path).with_context(|| format!("打开失败: {}", path.display()))?;
    let len = f.metadata()?.len();
    if len < TRAILER_SIZE + 16 {
        return Ok(None);
    }
    f.seek(SeekFrom::End(-(TRAILER_SIZE as i64)))?;
    let mut buf = [0u8; TRAILER_SIZE as usize];
    f.read_exact(&mut buf)?;
    let t = match Trailer::decode(&buf) {
        Some(t) => t,
        None => return Ok(None),
    };
    if !t.sane(len) {
        bail!("安装包 Trailer 校验失败（文件可能被截断或修改）");
    }
    Ok(Some(t))
}

/// 将 Trailer 追加到文件末尾。
pub fn write_trailer(f: &mut File, t: &Trailer) -> std::io::Result<()> {
    f.seek(SeekFrom::End(0))?;
    f.write_all(&t.encode())?;
    f.flush()
}

/// 从文件指定分区读取字节。
pub fn read_range(f: &mut File, r: Range) -> Result<Vec<u8>> {
    f.seek(SeekFrom::Start(r.off))?;
    let mut buf = vec![0u8; r.len as usize];
    f.read_exact(&mut buf)?;
    Ok(buf)
}

// ---------------------------------------------------------------------------
// 自包含卸载器（NSIS 式）：把卸载清单与应用图标内嵌进 uninstall.exe 尾部。
// 布局：[卸载器 EXE][manifest JSON][icon 字节][footer 24B]
// footer = magic "MIEDUNIN" (8) + manifest_len (8) + icon_len (8)
// ---------------------------------------------------------------------------

pub const UNINSTALL_MAGIC: &[u8; 8] = b"MIEDUNIN";
pub const UNINSTALL_FOOTER_SIZE: u64 = 24;

/// 将卸载清单与图标追加到卸载器 EXE 尾部。
pub fn write_uninstall_data(exe: &Path, manifest_json: &[u8], icon: &[u8]) -> std::io::Result<()> {
    let mut f = fs::OpenOptions::new().append(true).open(exe)?;
    f.write_all(manifest_json)?;
    f.write_all(icon)?;
    let mut footer = [0u8; UNINSTALL_FOOTER_SIZE as usize];
    footer[0..8].copy_from_slice(UNINSTALL_MAGIC);
    footer[8..16].copy_from_slice(&(manifest_json.len() as u64).to_le_bytes());
    footer[16..24].copy_from_slice(&(icon.len() as u64).to_le_bytes());
    f.write_all(&footer)?;
    f.flush()
}

/// 从卸载器 EXE 尾部读取（清单, 图标）；没有内嵌数据返回 None。
pub fn read_uninstall_data(exe: &Path) -> Option<(Vec<u8>, Vec<u8>)> {
    let mut f = File::open(exe).ok()?;
    let len = f.metadata().ok()?.len();
    if len < UNINSTALL_FOOTER_SIZE {
        return None;
    }
    f.seek(SeekFrom::End(-(UNINSTALL_FOOTER_SIZE as i64))).ok()?;
    let mut footer = [0u8; UNINSTALL_FOOTER_SIZE as usize];
    f.read_exact(&mut footer).ok()?;
    if &footer[0..8] != UNINSTALL_MAGIC {
        return None;
    }
    let manifest_len = u64::from_le_bytes(footer[8..16].try_into().ok()?);
    let icon_len = u64::from_le_bytes(footer[16..24].try_into().ok()?);
    let total = manifest_len.checked_add(icon_len)?;
    if total > len.saturating_sub(UNINSTALL_FOOTER_SIZE) {
        return None;
    }
    f.seek(SeekFrom::End(
        -((UNINSTALL_FOOTER_SIZE + total) as i64),
    ))
    .ok()?;
    let mut manifest = vec![0u8; manifest_len as usize];
    f.read_exact(&mut manifest).ok()?;
    let mut icon = vec![0u8; icon_len as usize];
    f.read_exact(&mut icon).ok()?;
    Some((manifest, icon))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode() {
        let t = Trailer {
            payload: Range { off: 16, len: 32 },
            config: Range { off: 48, len: 10 },
            uninstaller: Range { off: 58, len: 100 },
            background: Range::default(),
            icon: Range { off: 158, len: 3 },
            uninstall_icon: Range { off: 161, len: 0 },
        };
        let b = t.encode();
        let t2 = Trailer::decode(&b).unwrap();
        assert_eq!(t.payload, t2.payload);
        assert_eq!(t.config, t2.config);
        assert_eq!(t.uninstaller, t2.uninstaller);
        assert!(t2.background.is_empty());
        assert_eq!(t.icon, t2.icon);
    }
}
