//! MIEdit 归档格式（MIEditARC1）。
//!
//! 布局：
//! ```text
//! magic "MIEditARC1" (8B)
//! total_orig: u64          // 所有文件未压缩总字节数（用于进度）
//! 重复条目直到 path_len == 0：
//!   path_len: u16
//!   path: UTF-8，使用 '/' 分隔
//!   flags: u8              // bit0=目录, bit1=deflate 压缩
//!   comp_len: u64
//!   orig_len: u64
//!   data: comp_len 字节
//! terminator: u16 = 0
//! ```

use anyhow::{bail, Context, Result};
use flate2::read::DeflateDecoder;
use flate2::{read::DeflateEncoder, Compression};
use std::io::{Read, Write};

pub const ARCHIVE_MAGIC: &[u8; 8] = b"MIEDARC2";
pub const FLAG_DIR: u8 = 1;
pub const FLAG_DEFLATE: u8 = 2;

/// 顺序写入归档。调用前必须已知未压缩总字节数。
pub struct ArchiveWriter<W: Write> {
    inner: W,
    entries: u64,
}

impl<W: Write> ArchiveWriter<W> {
    /// `file_count`：文件总数（不含目录），供安装页显示“已写入 x/n 个文件”。
    pub fn new_with_total(mut inner: W, total_orig: u64, file_count: u64) -> std::io::Result<Self> {
        inner.write_all(ARCHIVE_MAGIC)?;
        inner.write_all(&total_orig.to_le_bytes())?;
        inner.write_all(&file_count.to_le_bytes())?;
        Ok(Self { inner, entries: 0 })
    }

    pub fn add_dir(&mut self, path: &str) -> std::io::Result<()> {
        self.write_header(path, FLAG_DIR, 0, 0)?;
        self.entries += 1;
        Ok(())
    }

    /// 写入一个文件。压缩后若未变小则原样存储。
    pub fn add_file(&mut self, path: &str, data: &[u8]) -> std::io::Result<()> {
        let mut comp = Vec::with_capacity(data.len() / 2 + 64);
        DeflateEncoder::new(&data[..], Compression::best()).read_to_end(&mut comp)?;
        let (flag, payload): (u8, &[u8]) = if comp.len() < data.len() {
            (FLAG_DEFLATE, &comp)
        } else {
            (0, data)
        };
        self.write_header(path, flag, payload.len() as u64, data.len() as u64)?;
        self.inner.write_all(payload)?;
        self.entries += 1;
        Ok(())
    }

    /// 写入一个文件（不压缩，原样存储）——供 LZMA solid 整包压缩使用，
    /// 让整包压缩器看到原始数据以获得最佳压缩率（NSIS solid 做法）。
    pub fn add_file_stored(&mut self, path: &str, data: &[u8]) -> std::io::Result<()> {
        self.write_header(path, 0, data.len() as u64, data.len() as u64)?;
        self.inner.write_all(data)?;
        self.entries += 1;
        Ok(())
    }

    fn write_header(&mut self, path: &str, flag: u8, comp: u64, orig: u64) -> std::io::Result<()> {
        let p = path.as_bytes();
        if p.len() > u16::MAX as usize {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "路径过长",
            ));
        }
        self.inner.write_all(&(p.len() as u16).to_le_bytes())?;
        self.inner.write_all(p)?;
        self.inner.write_all(&[flag])?;
        self.inner.write_all(&comp.to_le_bytes())?;
        self.inner.write_all(&orig.to_le_bytes())
    }

    /// 写入结束标记并 flush。
    pub fn finish(mut self) -> std::io::Result<W> {
        self.inner.write_all(&0u16.to_le_bytes())?;
        self.inner.flush()?;
        Ok(self.inner)
    }

    pub fn entry_count(&self) -> u64 {
        self.entries
    }
}

/// 一个归档条目（数据已读入内存）。
pub struct ArchiveEntry {
    pub path: String,
    pub is_dir: bool,
    pub orig_len: u64,
    data: Vec<u8>,
    compressed: bool,
}

impl ArchiveEntry {
    /// 解压得到原始数据（目录返回空）。
    pub fn decode(&self) -> Result<Vec<u8>> {
        if self.is_dir {
            return Ok(Vec::new());
        }
        if self.compressed {
            let mut out = Vec::with_capacity(self.orig_len as usize);
            DeflateDecoder::new(&self.data[..])
                .read_to_end(&mut out)
                .context("解压数据失败")?;
            if out.len() as u64 != self.orig_len {
                bail!(
                    "解压后大小不符: {} (期望 {}, 实际 {})",
                    self.path,
                    self.orig_len,
                    out.len()
                );
            }
            Ok(out)
        } else {
            if self.data.len() as u64 != self.orig_len {
                bail!("存储数据大小不符: {}", self.path);
            }
            Ok(self.data.clone())
        }
    }
}

/// 顺序读取归档。
pub struct ArchiveReader<R: Read> {
    inner: R,
    total_orig: u64,
    file_count: u64,
}

impl<R: Read> ArchiveReader<R> {
    pub fn new(mut inner: R) -> Result<Self> {
        let mut magic = [0u8; 8];
        inner.read_exact(&mut magic).context("读取归档头失败")?;
        if &magic != ARCHIVE_MAGIC {
            bail!("无效的 MIEdit 归档（magic 不符）");
        }
        let mut tb = [0u8; 8];
        inner.read_exact(&mut tb)?;
        let mut cb = [0u8; 8];
        inner.read_exact(&mut cb)?;
        Ok(Self {
            inner,
            total_orig: u64::from_le_bytes(tb),
            file_count: u64::from_le_bytes(cb),
        })
    }

    /// 归档中所有文件的未压缩总字节数。
    pub fn total_bytes(&self) -> u64 {
        self.total_orig
    }

    /// 归档中文件总数（不含目录）。
    pub fn file_count(&self) -> u64 {
        self.file_count
    }

    /// 读取下一个条目；返回 None 表示结束。
    pub fn next_entry(&mut self) -> Result<Option<ArchiveEntry>> {
        let mut l2 = [0u8; 2];
        match self.inner.read_exact(&mut l2) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e.into()),
        }
        let plen = u16::from_le_bytes(l2) as usize;
        if plen == 0 {
            return Ok(None);
        }
        let mut pbuf = vec![0u8; plen];
        self.inner.read_exact(&mut pbuf)?;
        let path = String::from_utf8(pbuf).context("归档路径不是有效 UTF-8")?;
        let mut flag = [0u8; 1];
        self.inner.read_exact(&mut flag)?;
        let mut cl = [0u8; 8];
        self.inner.read_exact(&mut cl)?;
        let mut ol = [0u8; 8];
        self.inner.read_exact(&mut ol)?;
        let comp_len = u64::from_le_bytes(cl);
        let orig_len = u64::from_le_bytes(ol);
        let mut data = vec![0u8; comp_len as usize];
        self.inner.read_exact(&mut data)?;
        Ok(Some(ArchiveEntry {
            path,
            is_dir: flag[0] & FLAG_DIR != 0,
            orig_len,
            compressed: flag[0] & FLAG_DEFLATE != 0,
            data,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn roundtrip() {
        let mut buf = Cursor::new(Vec::new());
        let hello = b"hello world\nhello world";
        {
            let mut w =
                ArchiveWriter::new_with_total(&mut buf, hello.len() as u64 + 100_000, 2).unwrap();
            w.add_dir("docs").unwrap();
            w.add_file("hello.txt", hello).unwrap();
            w.add_file("a/big.bin", &[7u8; 100_000]).unwrap();
            w.finish().unwrap();
        }
        buf.set_position(0);
        let mut r = ArchiveReader::new(buf).unwrap();
        assert_eq!(r.total_bytes(), hello.len() as u64 + 100_000);
        assert_eq!(r.file_count(), 2);
        let e1 = r.next_entry().unwrap().unwrap();
        assert!(e1.is_dir && e1.path == "docs");
        let e2 = r.next_entry().unwrap().unwrap();
        assert_eq!(e2.decode().unwrap(), b"hello world\nhello world");
        let e3 = r.next_entry().unwrap().unwrap();
        let d3 = e3.decode().unwrap();
        assert_eq!(d3.len(), 100_000);
        assert!(d3.iter().all(|&b| b == 7));
        assert!(r.next_entry().unwrap().is_none());
    }
}
