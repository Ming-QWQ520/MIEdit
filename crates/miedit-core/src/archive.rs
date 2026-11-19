//! MIEdit 归档格式（MIEditARC1）。
//!
//! 布局：
//! ```text
//! magic "MIEDARC2" (8B)
//! total_orig: u64          // 所有文件未压缩总字节数（用于进度）
//! file_count: u64          // 文件总数（不含目录，用于进度）
//! 重复条目直到 path_len == 0：
//!   path_len: u16
//!   path: UTF-8，使用 '/' 分隔
//!   flags: u8              // bit0=目录, bit1=deflate, bit2=brotli, bit3=lzma,
//!                          // bit4=bzip2, bit5=zstd（压缩位互斥，至多一个）
//!   comp_len: u64
//!   orig_len: u64
//!   data: comp_len 字节
//! terminator: u16 = 0
//! ```
//!
//! 压缩算法（NSIS SetCompressor 对齐 + 扩展，全部纯 Rust）：
//! - Deflate（zlib 等价，flate2/miniz_oxide，条目级与整包 solid 均支持）
//! - Brotli（brotli crate 纯 Rust 实现，比 Deflate 体积更小）
//! - BZip2（bzip2 0.6 crate，libbz2-rs 纯 Rust 实现，NSIS 经典算法）
//! - LZMA（lzma-rust 纯 Rust 实现，tukaani xz-for-java 移植，NSIS 旗舰格式）
//! - Zstd（mzstd 纯 Rust 实现，zstd 1.5.7 编码器逐字节等价移植，自带解码器）
//! - 激进压缩档（ultra）：LZMA Ultra = preset 9（64MB 字典）+ nice_len 273；
//!   Zstd Ultra = level 22。ultra 仅改变编码参数，输出格式与基础档一致
//!   （LZMA 头部/ zstd 帧自描述），解码端无需区分，旧安装器照常解码。
//!
//! 兼容性：旧包仅含 bit0/bit1，新安装器解码行为不变；新包含 bit2/bit3/bit4/bit5 时
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
pub const FLAG_BZIP2: u8 = 16;
pub const FLAG_ZSTD: u8 = 32;

/// 单条目长度上限（2 GiB，步骤 49 健壮性加固）：损坏归档的长度字段
/// （comp_len/orig_len）最大到 u64::MAX，直接按值分配会 OOM abort；
/// 超限改为优雅报错。合法安装包条目远低于此值。
pub const MAX_ENTRY_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// 压缩算法（条目级与整包 solid 共用，全部纯 Rust 实现）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Algo {
    /// Deflate（zlib 等价，flate2/miniz_oxide）
    Deflate,
    /// Brotli（纯 Rust，比 Deflate 体积更小）
    Brotli,
    /// LZMA1（lzma-rust，NSIS 旗舰压缩格式）
    Lzma,
    /// BZip2（bzip2 0.6 crate / libbz2-rs 纯 Rust，NSIS 经典算法）
    Bzip2,
    /// Zstd（mzstd 纯 Rust，zstd 1.5.7 逐字节等价编码器）
    Zstd,
}

impl Algo {
    /// 条目标志位（至多一个压缩位；bit0 独立表示目录）。
    pub fn flag(self) -> u8 {
        match self {
            Algo::Deflate => FLAG_DEFLATE,
            Algo::Brotli => FLAG_BROTLI,
            Algo::Lzma => FLAG_LZMA,
            Algo::Bzip2 => FLAG_BZIP2,
            Algo::Zstd => FLAG_ZSTD,
        }
    }

    /// 从条目标志位反解算法（bit0 目录位由调用方先行剥离）。
    pub fn from_flag(flag: u8) -> Option<Algo> {
        match flag {
            FLAG_DEFLATE => Some(Algo::Deflate),
            FLAG_BROTLI => Some(Algo::Brotli),
            FLAG_LZMA => Some(Algo::Lzma),
            FLAG_BZIP2 => Some(Algo::Bzip2),
            FLAG_ZSTD => Some(Algo::Zstd),
            _ => None,
        }
    }

    /// 人类可读名称（进度/日志文案用）。
    pub fn label(self) -> &'static str {
        match self {
            Algo::Deflate => "Deflate",
            Algo::Brotli => "Brotli",
            Algo::Lzma => "LZMA",
            Algo::Bzip2 => "BZip2",
            Algo::Zstd => "Zstd",
        }
    }
}

/// 归一化 `install.compression` 配置值（NSIS `SetCompressor zlib/bzip2/lzma(+solid)` 对齐 + 扩展）。
///
/// 返回规范值：`deflate`（条目级 Deflate）、`solid`（整包 Deflate，NSIS solid 风格）、
/// `brotli`（条目级 Brotli）、`brotli-solid`（整包 Brotli）、`bzip2`（条目级 BZip2）、
/// `bzip2-solid`（整包 BZip2）、`lzma`（条目级 LZMA）、`lzma-solid`（整包 LZMA）、
/// `lzma-ultra`（条目级 LZMA 激进档）、`lzma-ultra-solid`（整包 LZMA 激进档）、
/// `zstd`（条目级 Zstd）、`zstd-solid`（整包 Zstd）、`zstd-ultra`（条目级 Zstd 激进档）、
/// `zstd-ultra-solid`（整包 Zstd 激进档）。历史值与常见别名（下划线/斜杠变体）全部兼容。
pub fn normalize_compression(input: &str) -> Result<&'static str> {
    let v = input.trim().to_ascii_lowercase().replace('_', "-");
    match v.as_str() {
        "" | "deflate" | "zlib" | "zlib/deflate" => Ok("deflate"),
        "solid" | "deflate-solid" | "zlib-solid" | "zlib/solid" => Ok("solid"),
        "brotli" => Ok("brotli"),
        "brotli-solid" | "brotlisolid" => Ok("brotli-solid"),
        "bzip2" | "bz2" | "bzip" => Ok("bzip2"),
        "bzip2-solid" | "bz2-solid" | "bzip-solid" | "bzip2solid" => Ok("bzip2-solid"),
        "lzma" | "lzma1" => Ok("lzma"),
        "lzma-solid" | "lzmasolid" | "lzma1-solid" => Ok("lzma-solid"),
        "lzma-ultra" | "lzma2-ultra" | "lzma1-ultra" | "lzma-9" | "lzma-ultra2" => Ok("lzma-ultra"),
        "lzma-ultra-solid" | "lzma2-ultra-solid" | "lzma1-ultra-solid" | "lzma-9-solid" => {
            Ok("lzma-ultra-solid")
        }
        "zstd" | "zstandard" | "zst" => Ok("zstd"),
        "zstd-solid" | "zstandard-solid" | "zst-solid" | "zstdsolid" => Ok("zstd-solid"),
        "zstd-ultra" | "zstd-22" | "zst-ultra" => Ok("zstd-ultra"),
        "zstd-ultra-solid" | "zstd-22-solid" | "zst-ultra-solid" => Ok("zstd-ultra-solid"),
        other => bail!(
            "未知压缩方式 \"{}\"（支持: deflate, solid, lzma, lzma-solid, lzma-ultra, \
             lzma-ultra-solid, brotli, brotli-solid, bzip2, bzip2-solid, zstd, \
             zstd-solid, zstd-ultra, zstd-ultra-solid）",
            other
        ),
    }
}

/// 规范压缩值 → `(solid 整包, 算法, ultra 激进档)`。
///
/// 未知值返回 `None`：调用方（安装器）按条目级兼容处理，编译器在归一化之后调用
/// 不会命中 `None`（防御式兜底）。
pub fn compression_params(canonical: &str) -> Option<(bool, Algo, bool)> {
    match canonical {
        "deflate" => Some((false, Algo::Deflate, false)),
        "solid" => Some((true, Algo::Deflate, false)),
        "brotli" => Some((false, Algo::Brotli, false)),
        "brotli-solid" => Some((true, Algo::Brotli, false)),
        "bzip2" => Some((false, Algo::Bzip2, false)),
        "bzip2-solid" => Some((true, Algo::Bzip2, false)),
        "lzma" => Some((false, Algo::Lzma, false)),
        "lzma-solid" => Some((true, Algo::Lzma, false)),
        "lzma-ultra" => Some((false, Algo::Lzma, true)),
        "lzma-ultra-solid" => Some((true, Algo::Lzma, true)),
        "zstd" => Some((false, Algo::Zstd, false)),
        "zstd-solid" => Some((true, Algo::Zstd, false)),
        "zstd-ultra" => Some((false, Algo::Zstd, true)),
        "zstd-ultra-solid" => Some((true, Algo::Zstd, true)),
        _ => None,
    }
}

/// 规范压缩值 → 用户界面文案（builder 下拉/预览摘要共用，避免多处重复映射）。
/// 未知值/空值回退 Deflate 默认文案（与历史默认档一致）。
pub fn compression_label(canonical: &str) -> &'static str {
    match canonical {
        "solid" => "Deflate Solid（整包，体积更小）",
        "brotli" => "Brotli（条目级，现代算法）",
        "brotli-solid" => "Brotli Solid（整包，现代算法）",
        "bzip2" => "BZip2（条目级，NSIS 经典算法）",
        "bzip2-solid" => "BZip2 Solid（整包，NSIS 经典算法）",
        "lzma" => "LZMA（条目级，NSIS 旗舰算法）",
        "lzma-solid" => "LZMA Solid（整包，NSIS 默认风格，体积最小）",
        "lzma-ultra" => "LZMA Ultra（条目级，激进压缩 64MB 字典）",
        "lzma-ultra-solid" => "LZMA Ultra Solid（整包，激进压缩）",
        "zstd" => "Zstd（条目级，现代高性能）",
        "zstd-solid" => "Zstd Solid（整包，现代高性能）",
        "zstd-ultra" => "Zstd Ultra（条目级，激进压缩 level 22）",
        "zstd-ultra-solid" => "Zstd Ultra Solid（整包，激进压缩）",
        _ => "Deflate（条目级，速度快）",
    }
}

/// 下拉框展示顺序（builder 压缩选择框共用）。
pub const COMPRESSION_OPTIONS: &[&str] = &[
    "deflate",
    "solid",
    "brotli",
    "brotli-solid",
    "bzip2",
    "bzip2-solid",
    "lzma",
    "lzma-solid",
    "lzma-ultra",
    "lzma-ultra-solid",
    "zstd",
    "zstd-solid",
    "zstd-ultra",
    "zstd-ultra-solid",
];

/// 一次性压缩（内存到内存，条目级与整包 solid 共用，标准档）。
pub fn compress_bytes(algo: Algo, data: &[u8]) -> Result<Vec<u8>> {
    compress_bytes_profile(algo, data, false)
}

/// 一次性压缩（可选拉激进 ultra 档）：
/// - LZMA：preset 9（64MB 字典）+ nice_len 273（xz -9e 同档）；标准档 preset 6（8MB 字典）。
/// - Zstd：ultra = level 22；标准档 level 19。
/// - Brotli：ultra = 质量 11；标准档质量 9。
/// - Deflate/BZip2：已是最高档（flate2 best / BZip2 level 9），ultra 无差异。
/// 输出格式与标准档完全一致（自描述头），解码端无需知道是否 ultra。
pub fn compress_bytes_profile(algo: Algo, data: &[u8], ultra: bool) -> Result<Vec<u8>> {
    match algo {
        Algo::Deflate => {
            let mut out = Vec::with_capacity(data.len() / 2 + 64);
            DeflateEncoder::new(data, Compression::best()).read_to_end(&mut out)?;
            Ok(out)
        }
        Algo::Brotli => {
            let mut params = brotli::enc::BrotliEncoderParams::default();
            // 质量 9：与 flate2 best() 同级的“高质量、可接受耗时”档；
            // ultra 拉满 11（耗时陡增，仅激进档使用）。
            params.quality = if ultra { 11 } else { 9 };
            let mut out = Vec::with_capacity(data.len() / 2 + 64);
            brotli::CompressorReader::with_params(data, 4096, &params).read_to_end(&mut out)?;
            Ok(out)
        }
        Algo::Lzma => {
            // NSIS lzma 档位：标准档 preset 6（8MB 字典）；激进档 preset 9（64MB 字典）
            // + nice_len 273（NICE_LEN_MAX，xz -9e 语义）。头部自述格式（props + dict_size +
            // uncomp_size），解码端 LZMAReader::new_mem_limit 读取同一头部，无需额外携带参数。
            let mut options = LZMA2Options::with_preset(if ultra { 9 } else { 6 });
            if ultra {
                options.nice_len = LZMA2Options::NICE_LEN_MAX;
            }
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
        Algo::Bzip2 => {
            // bzip2 level 9 = 900k 块大小（NSIS bzip2 最高档；libbz2-rs 纯 Rust 实现）。
            let mut out = Vec::with_capacity(data.len() / 2 + 64);
            bzip2::read::BzEncoder::new(data, bzip2::Compression::best()).read_to_end(&mut out)?;
            Ok(out)
        }
        Algo::Zstd => {
            // 标准档 level 19（xz -19 同档激进）；ultra 拉满 level 22（zstd --ultra -22，
            // 窗口 128MB，mzstd 解码端默认窗口上限 2^27 恰好覆盖）。
            let level = if ultra { 22 } else { 19 };
            let lz = mzstd::Level::try_from(level).context("zstd 压缩级别无效")?;
            let mut compressor = mzstd::Compressor::new(
                mzstd::EncoderConfig::default().with_level(lz),
            )
            .context("zstd 压缩器初始化失败")?;
            compressor
                .compress(data)
                .map_err(|e| anyhow::anyhow!("zstd 压缩失败: {e}"))
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
        Algo::Bzip2 => {
            let mut out = Vec::new();
            bzip2::read::BzDecoder::new(data).read_to_end(&mut out)?;
            Ok(out)
        }
        Algo::Zstd => {
            let mut decompressor = mzstd::Decompressor::new(mzstd::DecoderConfig::default());
            decompressor
                .decompress(data)
                .map_err(|e| anyhow::anyhow!("zstd 解压失败: {e}"))
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

    /// 写入一个文件：按指定算法压缩（标准档），压缩后未变小则原样存储。
    pub fn add_file_with_algo(
        &mut self,
        path: &str,
        data: &[u8],
        algo: Algo,
    ) -> std::io::Result<()> {
        self.add_file_with_algo_profile(path, data, algo, false)
    }

    /// 写入一个文件：按指定算法 + 激进档参数压缩，压缩后未变小则原样存储。
    /// ultra 仅影响编码参数（字典/质量级别），条目标志位与标准档一致。
    pub fn add_file_with_algo_profile(
        &mut self,
        path: &str,
        data: &[u8],
        algo: Algo,
        ultra: bool,
    ) -> std::io::Result<()> {
        let comp = compress_bytes_profile(algo, data, ultra).map_err(|e| {
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
        // 健壮性加固（步骤 49）：损坏包 / `--ncrc` 下条目长度字段是攻击面——
        // 直接 `vec![0u8; comp_len]` 会因 u64::MAX 级分配直接 OOM abort（不可
        // 恢复）。设 2GiB 单条目上限（远超合法安装包条目），超出即优雅报错，
        // 与 NSIS 逐条目校验长度的行为对齐。
        if comp_len > MAX_ENTRY_BYTES {
            bail!(
                "条目 {} 压缩长度 {} 超过上限 {}（归档损坏或非 MIEdit 归档）",
                plen,
                comp_len,
                MAX_ENTRY_BYTES
            );
        }
        if orig_len > MAX_ENTRY_BYTES {
            bail!(
                "条目 {} 原始长度 {} 超过上限 {}（归档损坏或非 MIEdit 归档）",
                plen, orig_len, MAX_ENTRY_BYTES
            );
        }
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
        assert_eq!(FLAG_BZIP2, 16);
        assert_eq!(FLAG_ZSTD, 32);
        assert_eq!(Algo::Deflate.flag(), FLAG_DEFLATE);
        assert_eq!(Algo::Brotli.flag(), FLAG_BROTLI);
        assert_eq!(Algo::Lzma.flag(), FLAG_LZMA);
        assert_eq!(Algo::Bzip2.flag(), FLAG_BZIP2);
        assert_eq!(Algo::Zstd.flag(), FLAG_ZSTD);
        assert_eq!(Algo::from_flag(FLAG_BROTLI), Some(Algo::Brotli));
        assert_eq!(Algo::from_flag(FLAG_BZIP2), Some(Algo::Bzip2));
        assert_eq!(Algo::from_flag(FLAG_ZSTD), Some(Algo::Zstd));
        assert_eq!(Algo::from_flag(0x40), None);
        assert_eq!(Algo::from_flag(FLAG_DEFLATE | FLAG_DIR), None);
    }

    /// 条目级五算法 roundtrip：压缩 → 归档 → 读回 → 解压 == 原数据。
    #[test]
    fn entry_roundtrip_all_algos() {
        let pattern = |i: usize| (i % 251) as u8; // 可压缩的非平凡模式
        let big: Vec<u8> = (0..100_000).map(pattern).collect();
        for algo in [
            Algo::Deflate,
            Algo::Brotli,
            Algo::Lzma,
            Algo::Bzip2,
            Algo::Zstd,
        ] {
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

    /// 整包 solid 五算法 roundtrip（编译器/安装器整包链路共用函数）。
    #[test]
    fn solid_roundtrip_all_algos() {
        let big: Vec<u8> = (0..200_000)
            .map(|i| if i % 3 == 0 { 0x55 } else { (i % 97) as u8 })
            .collect();
        for algo in [
            Algo::Deflate,
            Algo::Brotli,
            Algo::Lzma,
            Algo::Bzip2,
            Algo::Zstd,
        ] {
            let comp = compress_bytes(algo, &big).unwrap();
            assert!(comp.len() < big.len(), "{} 应有效压缩", algo.label());
            assert_eq!(decompress_bytes(algo, &comp).unwrap(), big);
        }
        // 空数据与不可压数据（压缩产物更大 → 归档层回退存储，但本函数自身不失败）
        for algo in [
            Algo::Deflate,
            Algo::Brotli,
            Algo::Lzma,
            Algo::Bzip2,
            Algo::Zstd,
        ] {
            assert_eq!(decompress_bytes(algo, &compress_bytes(algo, b"").unwrap()).unwrap(), b"");
            assert_eq!(
                decompress_bytes(algo, &compress_bytes(algo, &[9u8; 32]).unwrap()).unwrap(),
                &[9u8; 32]
            );
        }
    }

    /// 压缩比对照（Brotli/LZMA/BZip2/Zstd 应显著优于 Deflate，锁住“新增格式有真实价值”）。
    #[test]
    fn new_algos_compress_better_than_deflate_on_pattern() {
        let data: Vec<u8> = "MIEdit install package payload. ".repeat(4096).into_bytes(); // 128KB 重复文本
        let d = compress_bytes(Algo::Deflate, &data).unwrap();
        let b = compress_bytes(Algo::Brotli, &data).unwrap();
        let l = compress_bytes(Algo::Lzma, &data).unwrap();
        let bz = compress_bytes(Algo::Bzip2, &data).unwrap();
        let z = compress_bytes(Algo::Zstd, &data).unwrap();
        assert!(b.len() <= d.len(), "Brotli 不劣于 Deflate");
        assert!(l.len() <= d.len(), "LZMA 不劣于 Deflate");
        assert!(bz.len() < d.len(), "BZip2 应优于 Deflate（重复文本）");
        assert!(z.len() <= d.len(), "Zstd 不劣于 Deflate");
    }

    /// 规范压缩值 → (solid, 算法, ultra) 映射（编译器/安装器共用分派逻辑）。
    #[test]
    fn compression_params_mapping() {
        let cases: &[(&str, bool, Algo, bool)] = &[
            ("deflate", false, Algo::Deflate, false),
            ("solid", true, Algo::Deflate, false),
            ("brotli", false, Algo::Brotli, false),
            ("brotli-solid", true, Algo::Brotli, false),
            ("bzip2", false, Algo::Bzip2, false),
            ("bzip2-solid", true, Algo::Bzip2, false),
            ("lzma", false, Algo::Lzma, false),
            ("lzma-solid", true, Algo::Lzma, false),
            ("lzma-ultra", false, Algo::Lzma, true),
            ("lzma-ultra-solid", true, Algo::Lzma, true),
            ("zstd", false, Algo::Zstd, false),
            ("zstd-solid", true, Algo::Zstd, false),
            ("zstd-ultra", false, Algo::Zstd, true),
            ("zstd-ultra-solid", true, Algo::Zstd, true),
        ];
        for (v, solid, algo, ultra) in cases {
            assert_eq!(compression_params(v), Some((*solid, *algo, *ultra)), "{v}");
        }
        // 旧清单/未知值兼容：返回 None，安装器按条目级处理
        assert_eq!(compression_params(""), None);
        assert_eq!(compression_params("unknown"), None);
        // 全部下拉选项均可归一化且可映射
        for v in COMPRESSION_OPTIONS {
            let canon = normalize_compression(v).unwrap();
            assert!(compression_params(canon).is_some(), "{v}");
            let _ = compression_label(canon);
        }
    }

    /// 激进 ultra 档 roundtrip：LZMA preset 9 + nice_len 273 / Zstd level 22。
    /// 输出格式与标准档一致（自描述头），同一解码路径应可解。
    #[test]
    fn ultra_profile_roundtrip() {
        let big: Vec<u8> = (0..200_000)
            .map(|i| (i % 253) as u8 ^ ((i / 7) % 31) as u8)
            .collect();
        for (algo, ultra) in [(Algo::Lzma, true), (Algo::Zstd, true), (Algo::Brotli, true)] {
            let comp = compress_bytes_profile(algo, &big, ultra).unwrap();
            assert_eq!(decompress_bytes(algo, &comp).unwrap(), big, "{:?}", algo);
        }
        // 归档链路：ultra 条目写入后标志位与基础档一致，读回解码正常
        let mut buf = Cursor::new(Vec::new());
        {
            let mut w = ArchiveWriter::new_with_total(&mut buf, big.len() as u64, 2).unwrap();
            w.add_file_with_algo_profile("a/ultra.bin", &big, Algo::Lzma, true).unwrap();
            w.add_file_with_algo_profile("b/ultra.zst", &big, Algo::Zstd, true).unwrap();
            w.finish().unwrap();
        }
        buf.set_position(0);
        let mut r = ArchiveReader::new(buf).unwrap();
        for i in 0..2 {
            let e = r.next_entry().unwrap().unwrap();
            assert_eq!(e.decode().unwrap(), big, "entry {i}");
        }
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
        // 新增：BZip2 / Zstd / ultra（NSIS bzip2 对齐 + Inno zstd 对齐 + 激进档）
        assert_eq!(normalize_compression("bzip2").unwrap(), "bzip2");
        assert_eq!(normalize_compression("BZ2").unwrap(), "bzip2");
        assert_eq!(normalize_compression("bzip2_solid").unwrap(), "bzip2-solid");
        assert_eq!(normalize_compression("zstd").unwrap(), "zstd");
        assert_eq!(normalize_compression("ZStandard").unwrap(), "zstd");
        assert_eq!(normalize_compression("zst_solid").unwrap(), "zstd-solid");
        assert_eq!(normalize_compression("lzma-ultra").unwrap(), "lzma-ultra");
        assert_eq!(normalize_compression("lzma2_ultra").unwrap(), "lzma-ultra");
        assert_eq!(normalize_compression("lzma-9-solid").unwrap(), "lzma-ultra-solid");
        assert_eq!(normalize_compression("zstd_ultra").unwrap(), "zstd-ultra");
        assert_eq!(normalize_compression("zstd-22").unwrap(), "zstd-ultra");
        assert_eq!(normalize_compression("zstd_ultra_solid").unwrap(), "zstd-ultra-solid");
        assert!(normalize_compression("rar").is_err());
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

    /// 步骤 49 健壮性加固：损坏归档的 comp_len/orig_len 超限必须优雅报错
    /// （此前 `vec![0u8; comp_len]` 遇 u64::MAX 级字段直接 OOM abort，
    /// `--ncrc` 跳过 CRC 后是不可恢复的 DoS 面）。
    #[test]
    fn corrupted_entry_len_rejected_gracefully() {
        // 构造合法归档：magic(8) + total(8) + count(8) + path_len(2) + path + flag(1)
        // + comp_len(8) + orig_len(8) + data + terminator(2)
        let mut buf = Vec::new();
        {
            let mut w =
                ArchiveWriter::new_with_total(&mut buf, 4, 1).unwrap();
            w.add_file("x.txt", b"abcd").unwrap();
            w.finish().unwrap();
        }
        let patch_len_field = |buf: &[u8], value: u64| -> Vec<u8> {
            let mut b = buf.to_vec();
            let off = 8 + 8 + 8 + 2 + "x.txt".len() + 1; // comp_len 字段偏移
            b[off..off + 8].copy_from_slice(&value.to_le_bytes());
            b
        };
        // comp_len 超限（u64::MAX 与 3GiB 两档）
        for bad in [u64::MAX, MAX_ENTRY_BYTES + 1] {
            let mut r = ArchiveReader::new(Cursor::new(patch_len_field(&buf, bad))).unwrap();
            let err = r.next_entry().err().expect("超限 comp_len 必须返回 Err 而非 abort");
            assert!(err.to_string().contains("超过上限"), "{err}");
        }
        // orig_len 超限
        let mut b = buf.clone();
        let off = 8 + 8 + 8 + 2 + "x.txt".len() + 1 + 8;
        b[off..off + 8].copy_from_slice(&(MAX_ENTRY_BYTES + 1).to_le_bytes());
        let mut r = ArchiveReader::new(Cursor::new(b)).unwrap();
        let err = r.next_entry().err().expect("超限 orig_len 必须返回 Err 而非 abort");
        assert!(err.to_string().contains("超过上限"), "{err}");
        // 正常值不受影响
        let mut r2 = ArchiveReader::new(Cursor::new(buf)).unwrap();
        assert_eq!(r2.next_entry().unwrap().unwrap().decode().unwrap(), b"abcd");
    }
}
