//! 安装包尾（Trailer）。
//!
//! 成品安装包 EXE 布局：
//! ```text
//! [stub EXE] [payload 归档] [config JSON] [卸载器 EXE] [背景图] [图标] [卸载器壁纸] [Trailer]
//! ```
//!
//! Trailer 位于文件最末尾，四个版本（向后兼容，读取时自动识别）：
//! - v2（`MIEDPKG2`，104 字节）：magic(8) + 6 个 (off:u64, len:u64)；
//! - v3（`MIEDPKG3`，116 字节）：v2 内容 + payload/config/uninstaller 三个
//!   分区的 CRC32（u32 LE 各 4 字节）——NSIS `CRCCheck` 对齐（步骤 7）；
//! - v4（`MIEDPKG4`，132 字节）：v3 内容 + 卸载器壁纸分区（7 个 Range）——
//!   卸载壁纸对齐（步骤 19）。新包一律写 v4；旧格式包（v2/v3）仍可读取安装。
//!
//! 新包一律写 v4；旧格式包（v2/v3）仍可读取安装（校验跳过/壁纸为空）。

use crate::crc32::{crc32, Crc32};
use anyhow::{bail, Context, Result};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::fs;
use std::path::Path;

/// v3 魔数（带分区 CRC32 校验）
pub const TRAILER_MAGIC_V3: &[u8; 8] = b"MIEDPKG3";
/// v3 Trailer 大小：magic(8) + 6×16 + 3×4
pub const TRAILER_SIZE_V3: u64 = 116;
/// v4 魔数（v3 + 卸载器壁纸分区）
pub const TRAILER_MAGIC_V4: &[u8; 8] = b"MIEDPKG4";
/// v4 Trailer 大小：magic(8) + 7×16 + 3×4
pub const TRAILER_SIZE_V4: u64 = 132;
/// v2 魔数（旧格式，无 CRC；兼容保留）
pub const TRAILER_MAGIC: &[u8; 8] = b"MIEDPKG2";
/// v2 Trailer 大小：magic(8) + 6×16
pub const TRAILER_SIZE: u64 = 104;

/// 三个关键分区的 CRC32 校验值（v3 Trailer）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrcSet {
    pub payload: u32,
    pub config: u32,
    pub uninstaller: u32,
}

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
    /// 卸载器壁纸（可选，v4；缺省无壁纸走内置渐变）
    pub uninstall_bg: Range,
    /// 分区 CRC32（v3/v4 包）；None = 旧格式（v2，无校验）
    pub crc: Option<CrcSet>,
}

impl Trailer {
    /// 本 Trailer 对应的版本字节数。
    pub fn trailer_size(&self) -> u64 {
        if self.crc.is_some() {
            TRAILER_SIZE_V4
        } else {
            TRAILER_SIZE
        }
    }

    /// 编码为字节序列：有 CRC → MIEDPKG4（132B，v3 超集）；无 → MIEDPKG2（104B，旧格式字节级兼容）。
    pub fn encode(&self) -> Vec<u8> {
        let ranges = [
            self.payload,
            self.config,
            self.uninstaller,
            self.background,
            self.icon,
            self.uninstall_icon,
            self.uninstall_bg,
        ];
        let mut out = Vec::with_capacity(TRAILER_SIZE_V4 as usize);
        match self.crc {
            Some(c) => {
                out.extend_from_slice(TRAILER_MAGIC_V4);
                for r in &ranges {
                    out.extend_from_slice(&r.off.to_le_bytes());
                    out.extend_from_slice(&r.len.to_le_bytes());
                }
                out.extend_from_slice(&c.payload.to_le_bytes());
                out.extend_from_slice(&c.config.to_le_bytes());
                out.extend_from_slice(&c.uninstaller.to_le_bytes());
            }
            None => {
                out.extend_from_slice(TRAILER_MAGIC);
                for r in &ranges[..6] {
                    out.extend_from_slice(&r.off.to_le_bytes());
                    out.extend_from_slice(&r.len.to_le_bytes());
                }
            }
        }
        out
    }

    /// 按魔数解码 v4/v3/v2 字节序列；不认识则返回 None。
    pub fn decode(b: &[u8]) -> Option<Trailer> {
        let (ranges_n, has_crc) = if b.len() >= TRAILER_SIZE_V4 as usize && &b[0..8] == TRAILER_MAGIC_V4 {
            (7usize, true)
        } else if b.len() >= TRAILER_SIZE_V3 as usize && &b[0..8] == TRAILER_MAGIC_V3 {
            (6usize, true)
        } else if b.len() >= TRAILER_SIZE as usize && &b[0..8] == TRAILER_MAGIC {
            (6usize, false)
        } else {
            return None;
        };
        let mut pos = 8;
        let mut next = || {
            let off = u64::from_le_bytes(b[pos..pos + 8].try_into().unwrap());
            let len = u64::from_le_bytes(b[pos + 8..pos + 16].try_into().unwrap());
            pos += 16;
            Range { off, len }
        };
        let t = Trailer {
            payload: next(),
            config: next(),
            uninstaller: next(),
            background: next(),
            icon: next(),
            uninstall_icon: next(),
            uninstall_bg: if ranges_n == 7 {
                next()
            } else {
                Range::default()
            },
            crc: None,
        };
        if !has_crc {
            return Some(t);
        }
        // CRC 起点：v4 = 8 + 7*16 = 120；v3 = 8 + 6*16 = 104
        let crc_at = if ranges_n == 7 { 120usize } else { 104usize };
        let crc = CrcSet {
            payload: u32::from_le_bytes(b[crc_at..crc_at + 4].try_into().unwrap()),
            config: u32::from_le_bytes(b[crc_at + 4..crc_at + 8].try_into().unwrap()),
            uninstaller: u32::from_le_bytes(b[crc_at + 8..crc_at + 12].try_into().unwrap()),
        };
        Some(Trailer { crc: Some(crc), ..t })
    }

    /// 校验所有分区都落在文件内。
    fn sane(&self, file_len: u64) -> bool {
        let limit = file_len.saturating_sub(self.trailer_size());
        [
            self.payload,
            self.config,
            self.uninstaller,
            self.background,
            self.icon,
            self.uninstall_icon,
            self.uninstall_bg,
        ]
        .iter()
            .all(|r| r.end() <= limit && (r.len == 0 || r.off < limit))
    }
}

/// 读取文件末尾的 Trailer（v4 优先，v3/v2 回退）；没有则返回 None。
pub fn read_trailer(path: &Path) -> Result<Option<Trailer>> {
    let mut f = File::open(path).with_context(|| format!("打开失败: {}", path.display()))?;
    let len = f.metadata()?.len();

    // v4（MIEDPKG4，v3 + 卸载器壁纸分区）
    if len >= TRAILER_SIZE_V4 + 16 {
        f.seek(SeekFrom::End(-(TRAILER_SIZE_V4 as i64)))?;
        let mut buf = [0u8; TRAILER_SIZE_V4 as usize];
        f.read_exact(&mut buf)?;
        if let Some(t) = Trailer::decode(&buf) {
            if !t.sane(len) {
                bail!("安装包 Trailer 校验失败（文件可能被截断或修改）");
            }
            return Ok(Some(t));
        }
    }

    // v3（MIEDPKG3，带 CRC32）
    if len >= TRAILER_SIZE_V3 + 16 {
        f.seek(SeekFrom::End(-(TRAILER_SIZE_V3 as i64)))?;
        let mut buf = [0u8; TRAILER_SIZE_V3 as usize];
        f.read_exact(&mut buf)?;
        if let Some(t) = Trailer::decode(&buf) {
            if !t.sane(len) {
                bail!("安装包 Trailer 校验失败（文件可能被截断或修改）");
            }
            return Ok(Some(t));
        }
    }

    // v2 回退（旧格式包仍可安装）
    if len >= TRAILER_SIZE + 16 {
        f.seek(SeekFrom::End(-(TRAILER_SIZE as i64)))?;
        let mut buf = [0u8; TRAILER_SIZE as usize];
        f.read_exact(&mut buf)?;
        if let Some(t) = Trailer::decode(&buf) {
            if !t.sane(len) {
                bail!("安装包 Trailer 校验失败（文件可能被截断或修改）");
            }
            return Ok(Some(t));
        }
    }
    Ok(None)
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

/// 流式校验 payload/config/uninstaller 分区 CRC32（NSIS `CRCCheck` 语义）。
///
/// 旧格式包（`crc = None`）直接通过；失败返回带分区名与期望/实际校验值的错误
/// （安装器以此弹出"安装包已损坏"；`--ncrc` 在调用方跳过本函数）。
pub fn verify_crc(path: &Path, t: &Trailer) -> Result<()> {
    let Some(crc) = t.crc else {
        return Ok(());
    };
    let mut f = File::open(path).with_context(|| format!("打开失败: {}", path.display()))?;
    let checks = [
        ("payload", t.payload, crc.payload),
        ("config", t.config, crc.config),
        ("uninstaller", t.uninstaller, crc.uninstaller),
    ];
    for (name, range, expect) in checks {
        if range.end() < range.off {
            bail!("安装包 Trailer 分区非法（{name}）");
        }
        f.seek(SeekFrom::Start(range.off))?;
        let mut h = Crc32::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut left = range.len;
        while left > 0 {
            let n = left.min(buf.len() as u64) as usize;
            f.read_exact(&mut buf[..n])?;
            h.update(&buf[..n]);
            left -= n as u64;
        }
        let actual = h.finish();
        if actual != expect {
            bail!(
                "安装包已损坏（CRC32 校验失败: {name} 分区，期望 {expect:08x}，实际 {actual:08x}）。\
                 安装包可能在传输中被截断或篡改，请重新获取；或使用 --ncrc 跳过校验。"
            );
        }
    }
    Ok(())
}

/// 为指定字节集合计算 CrcSet（编译器在写包时调用）。
pub fn compute_crc(payload: &[u8], config: &[u8], uninstaller: &[u8]) -> CrcSet {
    CrcSet {
        payload: crc32(payload),
        config: crc32(config),
        uninstaller: crc32(uninstaller),
    }
}

// ---------------------------------------------------------------------------
// 自包含卸载器（NSIS 式）：把卸载清单、图标、壁纸内嵌进 uninstall.exe 尾部。
// 布局：[卸载器 EXE][manifest JSON][icon 字节][bg 字节][footer]
// footer 新格式（32B）= magic "MIEDUNIN" (8) + manifest_len (8) + icon_len (8) + bg_len (8)
// footer 旧格式（24B）= magic "MIEDUNIN" (8) + manifest_len (8) + icon_len (8)
// 读取时先尝试新格式，回退旧格式（旧安装器生成的卸载器仍可读）。
// ---------------------------------------------------------------------------

pub const UNINSTALL_MAGIC: &[u8; 8] = b"MIEDUNIN";
/// 新格式 footer：3 个 u64 长度字段（manifest/icon/bg）
pub const UNINSTALL_FOOTER_SIZE: u64 = 32;
/// 旧格式 footer：2 个 u64 长度字段（manifest/icon，无壁纸）
pub const UNINSTALL_FOOTER_SIZE_V1: u64 = 24;

/// 将卸载清单、图标、壁纸追加到卸载器 EXE 尾部（新格式 32B footer）。
pub fn write_uninstall_data(exe: &Path, manifest_json: &[u8], icon: &[u8], bg: &[u8]) -> std::io::Result<()> {
    let mut f = fs::OpenOptions::new().append(true).open(exe)?;
    f.write_all(manifest_json)?;
    f.write_all(icon)?;
    if !bg.is_empty() {
        f.write_all(bg)?;
    }
    let mut footer = [0u8; UNINSTALL_FOOTER_SIZE as usize];
    footer[0..8].copy_from_slice(UNINSTALL_MAGIC);
    footer[8..16].copy_from_slice(&(manifest_json.len() as u64).to_le_bytes());
    footer[16..24].copy_from_slice(&(icon.len() as u64).to_le_bytes());
    footer[24..32].copy_from_slice(&(bg.len() as u64).to_le_bytes());
    f.write_all(&footer)?;
    f.flush()
}

/// 从卸载器 EXE 尾部读取（清单, 图标, 壁纸）；没有内嵌数据返回 None。
/// 新格式（32B footer）优先，旧格式（24B footer，壁纸返回空）回退。
pub fn read_uninstall_data(exe: &Path) -> Option<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let mut f = File::open(exe).ok()?;
    let len = f.metadata().ok()?.len();

    // 新格式：footer 32B，bg_len 在最后 8 字节
    if len >= UNINSTALL_FOOTER_SIZE {
        f.seek(SeekFrom::End(-(UNINSTALL_FOOTER_SIZE as i64))).ok()?;
        let mut footer = [0u8; UNINSTALL_FOOTER_SIZE as usize];
        f.read_exact(&mut footer).ok()?;
        if &footer[0..8] == UNINSTALL_MAGIC {
            let manifest_len = u64::from_le_bytes(footer[8..16].try_into().ok()?);
            let icon_len = u64::from_le_bytes(footer[16..24].try_into().ok()?);
            let bg_len = u64::from_le_bytes(footer[24..32].try_into().ok()?);
            let total = manifest_len.checked_add(icon_len)?.checked_add(bg_len)?;
            if total <= len.saturating_sub(UNINSTALL_FOOTER_SIZE) {
                f.seek(SeekFrom::End(-((UNINSTALL_FOOTER_SIZE + total) as i64))).ok()?;
                let mut manifest = vec![0u8; manifest_len as usize];
                f.read_exact(&mut manifest).ok()?;
                let mut icon = vec![0u8; icon_len as usize];
                f.read_exact(&mut icon).ok()?;
                let mut bg = vec![0u8; bg_len as usize];
                f.read_exact(&mut bg).ok()?;
                return Some((manifest, icon, bg));
            }
            return None;
        }
    }

    // 旧格式回退：footer 24B，无壁纸
    if len >= UNINSTALL_FOOTER_SIZE_V1 {
        f.seek(SeekFrom::End(-(UNINSTALL_FOOTER_SIZE_V1 as i64))).ok()?;
        let mut footer = [0u8; UNINSTALL_FOOTER_SIZE_V1 as usize];
        f.read_exact(&mut footer).ok()?;
        if &footer[0..8] != UNINSTALL_MAGIC {
            return None;
        }
        let manifest_len = u64::from_le_bytes(footer[8..16].try_into().ok()?);
        let icon_len = u64::from_le_bytes(footer[16..24].try_into().ok()?);
        let total = manifest_len.checked_add(icon_len)?;
        if total > len.saturating_sub(UNINSTALL_FOOTER_SIZE_V1) {
            return None;
        }
        f.seek(SeekFrom::End(-((UNINSTALL_FOOTER_SIZE_V1 + total) as i64))).ok()?;
        let mut manifest = vec![0u8; manifest_len as usize];
        f.read_exact(&mut manifest).ok()?;
        let mut icon = vec![0u8; icon_len as usize];
        f.read_exact(&mut icon).ok()?;
        Some((manifest, icon, Vec::new()))
    } else {
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn sample_trailer(with_crc: bool) -> Trailer {
        Trailer {
            payload: Range { off: 16, len: 32 },
            config: Range { off: 48, len: 10 },
            uninstaller: Range { off: 58, len: 100 },
            background: Range::default(),
            icon: Range { off: 158, len: 3 },
            uninstall_icon: Range { off: 161, len: 0 },
            uninstall_bg: Range::default(),
            crc: if with_crc {
                Some(CrcSet { payload: 0x1111_2222, config: 0x3333_4444, uninstaller: 0x5555_6666 })
            } else {
                None
            },
        }
    }

    #[test]
    fn encode_decode_v2() {
        let t = sample_trailer(false);
        let b = t.encode();
        assert_eq!(b.len(), TRAILER_SIZE as usize);
        assert_eq!(&b[0..8], TRAILER_MAGIC);
        let t2 = Trailer::decode(&b).unwrap();
        assert_eq!(t.payload, t2.payload);
        assert_eq!(t.config, t2.config);
        assert_eq!(t.uninstaller, t2.uninstaller);
        assert!(t2.background.is_empty());
        assert_eq!(t.icon, t2.icon);
        assert!(t2.crc.is_none(), "v2 无 CRC");
    }

    #[test]
    fn encode_decode_v4() {
        let t = sample_trailer(true);
        let b = t.encode();
        assert_eq!(b.len(), TRAILER_SIZE_V4 as usize);
        assert_eq!(&b[0..8], TRAILER_MAGIC_V4);
        let t2 = Trailer::decode(&b).unwrap();
        assert_eq!(t.payload, t2.payload);
        assert_eq!(t.icon, t2.icon);
        assert_eq!(t2.crc, sample_trailer(true).crc);
        assert!(t2.uninstall_bg.is_empty());
    }

    /// v4 携带卸载壁纸分区时编解码往返。
    #[test]
    fn encode_decode_v4_with_bg() {
        let mut t = sample_trailer(true);
        t.uninstall_bg = Range { off: 200, len: 48 };
        let t2 = Trailer::decode(&t.encode()).unwrap();
        assert_eq!(t2.uninstall_bg, Range { off: 200, len: 48 });
        assert_eq!(t2.trailer_size(), TRAILER_SIZE_V4);
    }

    /// 旧 v3 包（116B Trailer，无壁纸字段）仍可解码（兼容旧格式安装包）。
    #[test]
    fn decode_v3_still_readable() {
        // 手工构造 v3 字节：magic + 6 Range + 3 CRC
        let mut b = Vec::new();
        b.extend_from_slice(TRAILER_MAGIC_V3);
        for r in [
            Range { off: 16, len: 32 },
            Range { off: 48, len: 10 },
            Range { off: 58, len: 100 },
            Range::default(),
            Range { off: 158, len: 3 },
            Range::default(),
        ] {
            b.extend_from_slice(&r.off.to_le_bytes());
            b.extend_from_slice(&r.len.to_le_bytes());
        }
        b.extend_from_slice(&0x1111_2222u32.to_le_bytes());
        b.extend_from_slice(&0x3333_4444u32.to_le_bytes());
        b.extend_from_slice(&0x5555_6666u32.to_le_bytes());
        assert_eq!(b.len(), TRAILER_SIZE_V3 as usize);
        let t = Trailer::decode(&b).unwrap();
        assert_eq!(t.payload.off, 16);
        assert!(t.uninstall_bg.is_empty(), "v3 无壁纸字段 → 空 Range");
        assert_eq!(t.crc.map(|c| c.payload), Some(0x1111_2222));
    }

    #[test]
    fn decode_rejects_unknown_magic() {
        let mut b = sample_trailer(true).encode();
        b[0] = b'X';
        assert!(Trailer::decode(&b).is_none());
        assert!(Trailer::decode(&[0u8; 8]).is_none());
    }

    /// 在临时目录构造一个真实安装包文件（stub + 分区 + Trailer），供读写/校验测试。
    struct TempPkg {
        path: std::path::PathBuf,
    }
    impl TempPkg {
        fn new(with_crc: bool) -> Self {
            let path = std::env::temp_dir().join(format!(
                "miedit-test-pkg-{}-{}.bin",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let stub = vec![0xEEu8; 16]; // stub 区域（Trailer off=0 之后）
            let payload = vec![0xA5u8; 32];
            let config = b"{\"app\":{}}" .to_vec();
            let unp = vec![0x5Au8; 100];
            let icon = vec![0x11u8, 0x22, 0x33];
            let mut f = File::create(&path).unwrap();
            f.write_all(&stub).unwrap();
            let payload_off = 16u64;
            f.write_all(&payload).unwrap();
            let config_off = payload_off + payload.len() as u64;
            f.write_all(&config).unwrap();
            let unp_off = config_off + config.len() as u64;
            f.write_all(&unp).unwrap();
            let icon_off = unp_off + unp.len() as u64;
            f.write_all(&icon).unwrap();
            let t = Trailer {
                payload: Range { off: payload_off, len: payload.len() as u64 },
                config: Range { off: config_off, len: config.len() as u64 },
                uninstaller: Range { off: unp_off, len: unp.len() as u64 },
                background: Range::default(),
                icon: Range { off: icon_off, len: icon.len() as u64 },
                uninstall_icon: Range::default(),
                uninstall_bg: Range::default(),
                crc: if with_crc {
                    Some(compute_crc(&payload, &config, &unp))
                } else {
                    None
                },
            };
            write_trailer(&mut f, &t).unwrap();
            drop(f);
            TempPkg { path }
        }

        /// 篡改指定分区（payload=0 / config=1 / uninstaller=2）内的一个字节。
        fn corrupt(&self, part: usize) {
            let t = read_trailer(&self.path).unwrap().unwrap();
            let range = match part {
                0 => t.payload,
                1 => t.config,
                _ => t.uninstaller,
            };
            let mut f = fs::OpenOptions::new().write(true).open(&self.path).unwrap();
            f.seek(SeekFrom::Start(range.off + range.len / 2)).unwrap();
            f.write_all(&[0xFF]).unwrap();
        }
    }
    impl Drop for TempPkg {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    #[test]
    fn read_verify_v4_package() {
        let pkg = TempPkg::new(true);
        let t = read_trailer(&pkg.path).unwrap().unwrap();
        assert!(t.crc.is_some(), "v4 包应带 CRC");
        assert_eq!(t.trailer_size(), TRAILER_SIZE_V4);
        verify_crc(&pkg.path, &t).unwrap();
    }

    /// 新格式卸载数据（32B footer，含壁纸）写读往返。
    #[test]
    fn uninstall_data_roundtrip_with_bg() {
        let path = std::env::temp_dir().join(format!(
            "miedit-test-unin-{}-{}.exe",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, b"UNINSTUB").unwrap();
        write_uninstall_data(&path, b"{\"m\":1}", b"ICONBYTES", b"BGBYTES").unwrap();
        let (m, icon, bg) = read_uninstall_data(&path).unwrap();
        assert_eq!(m, b"{\"m\":1}");
        assert_eq!(icon, b"ICONBYTES");
        assert_eq!(bg, b"BGBYTES");
        std::fs::remove_file(&path).ok();
    }

    /// 空壁纸写读往返（bg_len = 0）。
    #[test]
    fn uninstall_data_roundtrip_no_bg() {
        let path = std::env::temp_dir().join(format!(
            "miedit-test-unin2-{}-{}.exe",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, b"UNINSTUB2").unwrap();
        write_uninstall_data(&path, b"MANIFEST", b"ICO", &[]).unwrap();
        let (m, icon, bg) = read_uninstall_data(&path).unwrap();
        assert_eq!(m, b"MANIFEST");
        assert_eq!(icon, b"ICO");
        assert!(bg.is_empty());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn corrupt_payload_fails_verify() {
        let pkg = TempPkg::new(true);
        let t = read_trailer(&pkg.path).unwrap().unwrap();
        pkg.corrupt(0);
        let err = verify_crc(&pkg.path, &t).unwrap_err().to_string();
        assert!(err.contains("payload"), "错误应指明分区: {err}");
        assert!(err.contains("安装包已损坏"), "{err}");
    }

    #[test]
    fn corrupt_config_and_uninstaller_fail_verify() {
        let pkg = TempPkg::new(true);
        let t = read_trailer(&pkg.path).unwrap().unwrap();
        pkg.corrupt(1);
        assert!(verify_crc(&pkg.path, &t).unwrap_err().to_string().contains("config"));
        let pkg2 = TempPkg::new(true);
        let t2 = read_trailer(&pkg2.path).unwrap().unwrap();
        pkg2.corrupt(2);
        assert!(
            verify_crc(&pkg2.path, &t2)
                .unwrap_err()
                .to_string()
                .contains("uninstaller")
        );
    }

    #[test]
    fn v2_package_still_readable() {
        let pkg = TempPkg::new(false);
        let t = read_trailer(&pkg.path).unwrap().unwrap();
        assert!(t.crc.is_none(), "旧格式包无 CRC");
        assert_eq!(t.trailer_size(), TRAILER_SIZE);
        verify_crc(&pkg.path, &t).unwrap(); // 无 CRC → 直接通过
    }
}
