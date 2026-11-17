//! MIEdit 归档格式（MIEditARC1）。
//!
//! 布局：
//! ```text
//! magic "MIEditARC1" (8B)
//! total_orig: u64          // 所有文件未压缩总字节数（用于进度）
//! file_count: u64          // 文件总数（不含目录，用于进度）
//! 重复条目直到 path_len == 0：
//!   path_len: u16
//!   path: UTF-8，使用 '/' 分隔
//!   flags: u8              // bit0=目录, bit1=deflate, bit2=brotli, bit3=lzma（至多一个压缩位）
//!   comp_len: u64
//!   orig_len: u64
//!   data: comp_len 字节
//! terminator: u16 = 0
//! ```
//!
//! 压缩算法（NSIS SetCompressor 对齐 + 扩展，全部纯 Rust）：
//! - Deflate（zlib 等价，flate2/miniz_oxide，条目级与整包 solid 均支持）
//! - Brotli（brotli crate 纯 Rust 实现，比 Deflate 体积更小）
//! - LZMA（lzma-rust 纯 Rust 实现，tukaani xz-for-java 移植，NSIS 旗舰格式；
//!   条目级与整包 lzma-solid 均支持）
//!
//! 兼容性：旧包仅含 bit0/bit1，新安装器解码行为不变；新包含 bit2/bit3 时
//! 旧安装器无法解码（与 NSIS 一致：lzma 包需要带 lzma 支持的解压 stub）。

use anyhow::{bail, Context, Result};
use flate2::read::DeflateDecoder;
use flate2::{read::DeflateEncoder, Compression};
use lzma_rust::{CountingWriter, LZMA2Options, LZMAReader, LZMAWriter};
use std::io::{Read, Write};

pub const ARCHIVE_MAGIC: &[u8; 8] = b"MIEDARC2";
pub const FLAG_DIR: u8 = 1;
pub const FLAG_DEFLATE: u8 = 2;
pub const FLAG_BROTLI: u8 = 4;
pub const FLAG_LZMA: u8 = 8;

/// 压缩算法（条目级与整包 solid 共用，全部纯 Rust 实现）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Algo {
    /// Deflate（zlib 等价，flate2/miniz_oxide）
    Deflate,
    /// Brotli（纯 Rust，比 Deflate 体积更小）
    Brotli,
    /// LZMA1（lzma-rust，NSIS 旗舰压缩格式）
    Lzma,
}

impl Algo {
    /// 条目标志位（至多一个压缩位；bit0 独立表示目录）。
    pub fn flag(self) -> u8 {
        match self {
            Algo::Deflate => FLAG_DEFLATE,
            Algo::Brotli => FLAG_BROTLI,
            Algo::Lzma => FLAG_LZMA,
        }
    }

    /// 从条目标志位反解算法（bit0 目录位由调用方先行剥离）。
    pub fn from_flag(flag: u8) -> Option<Algo> {
        match flag {
            FLAG_DEFLATE => Some(Algo::Deflate),
            FLAG_BROTLI => Some(Algo::Brotli),
            FLAG_LZMA => Some(Algo::Lzma),
            _ => None,
        }
    }

    /// 人类可读名称（进度/日志文案用）。
    pub fn label(self) -> &'static str {
        match self {
            Algo::Deflate => "Deflate",
            Algo::Brotli => "Brotli",
            Algo::Lzma => "LZMA",
        }
    }
}

/// 归一化 `install.compression` 配置值（NSIS `SetCompressor zlib/lzma(+solid)` 对齐 + 扩展）。
///
/// 返回规范值：`deflate`（条目级 Deflate）、`solid`（整包 Deflate，NSIS solid 风格）、
/// `lzma`（条目级 LZMA）、`lzma-solid`（整包 LZMA）、`brotli`（条目级 Brotli）、
/// `brotli-solid`（整包 Brotli）。历史值与常见别名（下划线/斜杠变体）全部兼容。
pub fn normalize_compression(input: &str) -> Result<&'static str> {
    let v = input.trim().to_ascii_lowercase().replace('_', "-");
    match v.as_str() {
        "" | "deflate" | "zlib" | "zlib/deflate" => Ok("deflate"),
        "solid" | "deflate-solid" | "zlib-solid" | "zlib/solid" => Ok("solid"),
        "lzma" | "lzma1" => Ok("lzma"),
        "lzma-solid" | "lzmasolid" | "lzma1-solid" => Ok("lzma-solid"),
        "brotli" => Ok("brotli"),
        "brotli-solid" | "brotlisolid" => Ok("brotli-solid"),
        other => bail!(
            "未知压缩方式 \"{}\"（支持: deflate, solid, lzma, lzma-solid, brotli, \
             brotli-solid；zstd/bzip2 因纯 Rust 生态无成熟编码器暂不支持，见路线图步骤 16）",
            other
        ),
    }
}

/// 一次性压缩（内存到内存，条目级与整包 solid 共用）。
pub fn compress_bytes(algo: Algo, data: &[u8]) -> Result<Vec<u8>> {
    match algo {
        Algo::Deflate => {
            let mut out = Vec::with_capacity(data.len() / 2 + 64);
            DeflateEncoder::new(data, Compression::best()).read_to_end(&mut out)?;
            Ok(out)
        }
        Algo::Brotli => {
            let mut params = brotli::enc::BrotliEncoderParams::default();
            // 质量 9：与 flate2 best() 同级的“高质量、可接受耗时”档（11 增益有限但耗时陡增）。
            params.quality = 9;
            let mut out = Vec::with_capacity(data.len() / 2 + 64);
            brotli::CompressorReader::with_params(data, 4096, &params).read_to_end(&mut out)?;
            Ok(out)
        }
        Algo::Lzma => {
            // NSIS lzma 档位：preset 6（8MB 字典）。头部自述格式（props + dict_size +
            // uncomp_size），解码端 LZMAReader::new_mem_limit 读取同一头部，无需额外携带参数。
            let options = LZMA2Options::with_preset(6);
            let mut out = Vec::with_capacity(data.len() / 2 + 64);
            let mut w = LZMAWriter::new_use_header(
                CountingWriter::new(&mut out),
                &options,
                Some(data.len() as u64),
            )?;
            w.write_all(data)?;
            w.finish()?;
            Ok(out)
        }
    }
}

/// 一次性解压（内存到内存，条目级与整包 solid 共用）。
pub fn decompress_bytes(algo: Algo, data: &[u8]) -> Result<Vec<u8>> {
    match algo {
        Algo::Deflate => {
            let mut out = Vec::new();
            DeflateDecoder::new(data).read_to_end(&mut out)?;
            Ok(out)
        }
        Algo::Brotli => {
            let mut out = Vec::new();
            brotli::Decompressor::new(data, 4096).read_to_end(&mut out)?;
            Ok(out)
        }
        Algo::Lzma => {
            let mut out = Vec::new();
            LZMAReader::new_mem_limit(data, u32::MAX, None)?.read_to_end(&mut out)?;
            Ok(out)
        }
    }
}

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

    /// 写入一个文件（条目级 Deflate，历史默认档）。压缩后若未变小则原样存储。
    pub fn add_file(&mut self, path: &str, data: &[u8]) -> std::io::Result<()> {
        self.add_file_with_algo(path, data, Algo::Deflate)
    }

    /// 写入一个文件：按指定算法压缩，压缩后未变小则原样存储。
    pub fn add_file_with_algo(
        &mut self,
        path: &str,
        data: &[u8],
        algo: Algo,
    ) -> std::io::Result<()> {
        let comp = compress_bytes(algo, data).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, format!("压缩失败: {e:#}"))
        })?;
        let (flag, payload): (u8, Vec<u8>) = if !data.is_empty() && comp.len() < data.len() {
            (algo.flag(), comp)
        } else {
            (0, data.to_vec())
        };
        self.write_header(path, flag, payload.len() as u64, data.len() as u64)?;
        self.inner.write_all(&payload)?;
        self.entries += 1;
        Ok(())
    }

    /// 写入一个文件（不压缩，原样存储）——供 solid 整包压缩使用，
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
    /// 原始标志字节（bit0 目录位已存入 is_dir；压缩算法位见 [`Algo::from_flag`]）。
    flag: u8,
}

impl ArchiveEntry {
    /// 解压得到原始数据（目录返回空）。
    pub fn decode(&self) -> Result<Vec<u8>> {
        if self.is_dir {
            return Ok(Vec::new());
        }
        if self.flag == 0 {
            if self.data.len() as u64 != self.orig_len {
                bail!("存储数据大小不符: {}", self.path);
            }
            return Ok(self.data.clone());
        }
        let algo = Algo::from_flag(self.flag).with_context(|| {
            format!(
                "未知压缩标志 {:#04x}（条目 {}）——需要新版本安装器",
                self.flag, self.path
            )
        })?;
        let out = decompress_bytes(algo, &self.data)
            .with_context(|| format!("解压数据失败（{}）", algo.label()))?;
        if out.len() as u64 != self.orig_len {
            bail!(
                "解压后大小不符: {} (期望 {}, 实际 {})",
                self.path,
                self.orig_len,
                out.len()
            );
        }
        Ok(out)
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
            flag: flag[0],
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

    #[test]
    fn flag_bits_unchanged() {
        // 兼容性锚点：历史包格式标志位不得漂移
        assert_eq!(FLAG_DIR, 1);
        assert_eq!(FLAG_DEFLATE, 2);
        assert_eq!(FLAG_BROTLI, 4);
        assert_eq!(FLAG_LZMA, 8);
        assert_eq!(Algo::Deflate.flag(), FLAG_DEFLATE);
        assert_eq!(Algo::Brotli.flag(), FLAG_BROTLI);
        assert_eq!(Algo::Lzma.flag(), FLAG_LZMA);
        assert_eq!(Algo::from_flag(FLAG_BROTLI), Some(Algo::Brotli));
        assert_eq!(Algo::from_flag(0x10), None);
        assert_eq!(Algo::from_flag(FLAG_DEFLATE | FLAG_DIR), None);
    }

    /// 条目级三种算法 roundtrip：压缩 → 归档 → 读回 → 解压 == 原数据。
    #[test]
    fn entry_roundtrip_all_algos() {
        let pattern = |i: usize| (i % 251) as u8; // 可压缩的非平凡模式
        let big: Vec<u8> = (0..100_000).map(pattern).collect();
        for algo in [Algo::Deflate, Algo::Brotli, Algo::Lzma] {
            let mut buf = Cursor::new(Vec::new());
            {
                let mut w = ArchiveWriter::new_with_total(&mut buf, big.len() as u64, 2).unwrap();
                w.add_file_with_algo("a/big.bin", &big, algo).unwrap();
                w.add_file_with_algo("tiny.txt", b"abcabcabc", algo).unwrap();
                w.finish().unwrap();
            }
            buf.set_position(0);
            let mut r = ArchiveReader::new(buf).unwrap();
            let e1 = r.next_entry().unwrap().unwrap();
            assert_eq!(e1.decode().unwrap(), big, "algo={:?}", algo);
            let e2 = r.next_entry().unwrap().unwrap();
            assert_eq!(e2.decode().unwrap(), b"abcabcabc", "algo={:?}", algo);
        }
    }

    /// 整包 solid 三种算法 roundtrip（编译器/安装器整包链路共用函数）。
    #[test]
    fn solid_roundtrip_all_algos() {
        let big: Vec<u8> = (0..200_000)
            .map(|i| if i % 3 == 0 { 0x55 } else { (i % 97) as u8 })
            .collect();
        for algo in [Algo::Deflate, Algo::Brotli, Algo::Lzma] {
            let comp = compress_bytes(algo, &big).unwrap();
            assert!(comp.len() < big.len(), "{} 应有效压缩", algo.label());
            assert_eq!(decompress_bytes(algo, &comp).unwrap(), big);
        }
        // 空数据与不可压数据（压缩产物更大 → 归档层回退存储，但本函数自身不失败）
        for algo in [Algo::Deflate, Algo::Brotli, Algo::Lzma] {
            assert_eq!(decompress_bytes(algo, &compress_bytes(algo, b"").unwrap()).unwrap(), b"");
            assert_eq!(
                decompress_bytes(algo, &compress_bytes(algo, &[9u8; 32]).unwrap()).unwrap(),
                &[9u8; 32]
            );
        }
    }

    /// 压缩比对照（Brotli/LZMA 应显著优于 Deflate，锁住“新增格式有真实价值”）。
    #[test]
    fn new_algos_compress_better_than_deflate_on_pattern() {
        let data: Vec<u8> = "MIEdit install package payload. ".repeat(4096).into_bytes(); // 128KB 重复文本
        let d = compress_bytes(Algo::Deflate, &data).unwrap();
        let b = compress_bytes(Algo::Brotli, &data).unwrap();
        let l = compress_bytes(Algo::Lzma, &data).unwrap();
        assert!(b.len() <= d.len(), "Brotli 不劣于 Deflate");
        assert!(l.len() <= d.len(), "LZMA 不劣于 Deflate");
    }

    #[test]
    fn normalize_compression_aliases() {
        // 历史值与 NSIS SetCompressor 对齐
        assert_eq!(normalize_compression("").unwrap(), "deflate");
        assert_eq!(normalize_compression("deflate").unwrap(), "deflate");
        assert_eq!(normalize_compression(" solid ").unwrap(), "solid");
        assert_eq!(normalize_compression("deflate-solid").unwrap(), "solid");
        assert_eq!(normalize_compression("zlib-solid").unwrap(), "solid");
        assert_eq!(normalize_compression("lzma").unwrap(), "lzma");
        assert_eq!(normalize_compression("LZMA-Solid").unwrap(), "lzma-solid");
        assert_eq!(normalize_compression("lzma1").unwrap(), "lzma");
        assert_eq!(normalize_compression("brotli").unwrap(), "brotli");
        assert_eq!(normalize_compression("brotli_solid").unwrap(), "brotli-solid");
        assert!(normalize_compression("bzip2").is_err());
        assert!(normalize_compression("zstd").is_err());
    }

    /// 归档兼容性：手写旧格式条目（仅 bit1 deflate）仍可解码。
    #[test]
    fn legacy_deflate_flag_still_decodes() {
        let mut buf = Cursor::new(Vec::new());
        let data = b"legacy entry payload".repeat(8);
        {
            let mut w = ArchiveWriter::new_with_total(&mut buf, data.len() as u64, 1).unwrap();
            w.add_file("old.txt", &data).unwrap(); // 走历史 add_file 路径
            w.finish().unwrap();
        }
        buf.set_position(0);
        let mut r = ArchiveReader::new(buf).unwrap();
        let e = r.next_entry().unwrap().unwrap();
        assert_eq!(e.decode().unwrap(), data);
    }
}
