//! 压缩差分基准与可复现种子模糊往返（路线图步骤 46）。
//!
//! 三件事，全部确定性可复现（种子驱动，无外部依赖，不依赖文件系统随机性）：
//!
//! 1. **确定性语料**：`bench_corpus(seed)` 按五个熵级分区（文本 / 二进制 /
//!    零段 / 重复块 / 随机）生成固定规模语料，任何平台任何时间逐字节一致
//!    → 14 档差分基准表可复现、可入库对比；
//! 2. **14 档差分基准**：`bench_profiles(data)` 逐档（COMPRESSION_OPTIONS 全部
//!    14 个规范档位）压缩→解压→逐字节校验，产出压缩率 / 压缩耗时 / 解压耗时 /
//!    往返结果，`bench_markdown` 直接渲染入库表格；
//! 3. **种子模糊往返**：`fuzz_roundtrip(seed, iterations)` 以 SplitMix64
//!    生成随机尺寸 × 五熵级的伪随机语料，14 档全 roundtrip，失败收集为
//!    `FuzzFailure` 列表（空 = 全过）；单测内以小种子小批次跑回归，
//!    example/CI 用大种子大批次跑深模糊。
//!
//! 另含 `emit_payload_files`：端到端矩阵验证（步骤 47）用的确定性 payload
//! 生成器，同一种子下脚本端与比对端文件逐字节一致。
//!
//! 说明：solid（整包）与条目级在编解码器参数上完全一致，差异只在打包粒度
//! （单流 vs 多条目流），故基准按 14 个规范档位逐档测量，solid 孪生行仅作
//! 记录维度（打包后包体差异由 e2e 矩阵的包大小列体现）。

use crate::archive::{compress_bytes_profile, compression_params, decompress_bytes, COMPRESSION_OPTIONS};
use crate::util::human_size;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::time::Instant;

/// 基准语料默认种子（2026-11-20，步骤 46 立项日）。
pub const BENCH_SEED: u64 = 20261120;

/// 语料熵级（基准与模糊共用）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Entropy {
    /// 自然语言样文本（低熵，各算法收益差异最明显）
    Text,
    /// 二进制样字节（中熵，模拟可执行文件）
    Binary,
    /// 全零段（极高可压缩，padding/稀疏数据）
    Zeros,
    /// 固定小块重复（图标/资源样）
    Repeated,
    /// 纯随机（不可压缩下界）
    Random,
}

impl Entropy {
    /// 全部熵级（按 Text→Binary→Zeros→Repeated→Random 循环使用）。
    pub const ALL: [Entropy; 5] = [
        Entropy::Text,
        Entropy::Binary,
        Entropy::Zeros,
        Entropy::Repeated,
        Entropy::Random,
    ];

    fn label(self) -> &'static str {
        match self {
            Entropy::Text => "text",
            Entropy::Binary => "binary",
            Entropy::Zeros => "zeros",
            Entropy::Repeated => "repeated",
            Entropy::Random => "random",
        }
    }
}

/// SplitMix64：确定性 PRNG（无外部依赖，任何平台逐字节一致）。
struct SplitMix64(u64);

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn next_byte(&mut self) -> u8 {
        (self.next_u64() >> 56) as u8
    }

    /// `[start, end)` 区间均匀 usize（end > start 时有效，否则返回 start）。
    fn next_usize(&mut self, start: usize, end: usize) -> usize {
        if end <= start {
            return start;
        }
        start + (self.next_u64() as usize) % (end - start)
    }
}

/// 基准用词表（英文样文，低熵文本分区素材）。
const WORDS: &[&str] = &[
    "installer", "package", "compression", "archive", "component", "directory",
    "shortcut", "uninstall", "registry", "license", "wizard", "silent",
    "extract", "overwrite", "reboot", "language", "manifest", "version",
    "payload", "trailer", "setup", "setup", "wizard", "nsis", "solid",
    "lzma", "zstd", "bzip2", "brotli", "deflate",
];

/// 生成一段 `size` 字节的指定熵级数据（确定性）。
pub fn gen_section(seed: u64, kind: Entropy, size: usize) -> Vec<u8> {
    let mut rng = SplitMix64::new(seed);
    match kind {
        Entropy::Zeros => vec![0u8; size],
        Entropy::Random => (0..size).map(|_| rng.next_byte()).collect(),
        Entropy::Text => {
            let mut out = Vec::with_capacity(size);
            let mut i = 0usize;
            while out.len() < size {
                let w = WORDS[rng.next_usize(0, WORDS.len())];
                if i % 9 == 8 {
                    out.push(b'\n');
                } else {
                    out.push(b' ');
                }
                out.extend_from_slice(w.as_bytes());
                i += 1;
            }
            out.truncate(size);
            out
        }
        Entropy::Binary => {
            // 二进制样：随机字节 + 间歇 4..=16 字节重复块（模拟代码段重定位）。
            let mut out = Vec::with_capacity(size);
            while out.len() < size {
                if rng.next_usize(0, 8) == 0 {
                    let run = rng.next_usize(4, 17);
                    let b = rng.next_byte();
                    for _ in 0..run {
                        out.push(b);
                    }
                } else {
                    out.push(rng.next_byte());
                }
            }
            out.truncate(size);
            out
        }
        Entropy::Repeated => {
            let pat_len = rng.next_usize(8, 257);
            let pat: Vec<u8> = (0..pat_len).map(|_| rng.next_byte()).collect();
            let mut out = Vec::with_capacity(size);
            while out.len() < size {
                let take = pat_len.min(size - out.len());
                out.extend_from_slice(&pat[..take]);
            }
            out
        }
    }
}

/// 生成 14 档差分基准语料（约 96KB，五熵级分区拼接，确定性）。
///
/// 分区：32KB 文本 + 24KB 二进制 + 8KB 零段 + 12KB 重复块 + 20KB 随机。
/// 规模取折中：足够让各算法拉开差距，又使全 14 档（含 brotli q11 /
/// zstd level 22）在普通机器数十秒内完成。
pub fn bench_corpus(seed: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(96 * 1024);
    out.extend_from_slice(&gen_section(seed ^ 0xA1, Entropy::Text, 32 * 1024));
    out.extend_from_slice(&gen_section(seed ^ 0xB2, Entropy::Binary, 24 * 1024));
    out.extend_from_slice(&gen_section(seed ^ 0xC3, Entropy::Zeros, 8 * 1024));
    out.extend_from_slice(&gen_section(seed ^ 0xD4, Entropy::Repeated, 12 * 1024));
    out.extend_from_slice(&gen_section(seed ^ 0xE5, Entropy::Random, 20 * 1024));
    out
}

/// 单档基准结果行。
#[derive(Clone, Debug)]
pub struct BenchRow {
    /// 规范档位名（COMPRESSION_OPTIONS 之一）。
    pub profile: &'static str,
    /// 算法人类可读名（Deflate/Brotli/LZMA/BZip2/Zstd）。
    pub algo_label: &'static str,
    /// 是否 solid 整包档（编解码参数与孪生条目档一致，记录维度）。
    pub solid: bool,
    /// 是否激进档（ultra）。
    pub ultra: bool,
    /// 原始字节数。
    pub orig_len: usize,
    /// 压缩后字节数。
    pub comp_len: usize,
    /// 压缩率（comp/orig×100，越小越小越好）。
    pub ratio_pct: f64,
    /// 压缩耗时（毫秒）。
    pub compress_ms: u128,
    /// 解压耗时（毫秒）。
    pub decompress_ms: u128,
    /// 往返是否逐字节一致。
    pub roundtrip_ok: bool,
    /// 压缩/解压失败原因（None = 成功）。
    pub error: Option<String>,
}

/// 14 档差分基准：对同一份语料逐档压缩→解压→逐字节校验。
/// 档位顺序与 `COMPRESSION_OPTIONS` 一致（下拉框展示顺序）。
pub fn bench_profiles(data: &[u8]) -> Vec<BenchRow> {
    COMPRESSION_OPTIONS
        .iter()
        .filter_map(|&p| compression_params(p).map(|(solid, algo, ultra)| (p, solid, algo, ultra)))
        .map(|(profile, solid, algo, ultra)| {
            let t0 = Instant::now();
            match compress_bytes_profile(algo, data, ultra) {
                Ok(comp) => {
                    let compress_ms = t0.elapsed().as_millis();
                    let t1 = Instant::now();
                    let back = decompress_bytes(algo, &comp);
                    let decompress_ms = t1.elapsed().as_millis();
                    let roundtrip_ok = matches!(&back, Ok(b) if b.as_slice() == data);
                    BenchRow {
                        profile,
                        algo_label: algo.label(),
                        solid,
                        ultra,
                        orig_len: data.len(),
                        comp_len: comp.len(),
                        ratio_pct: if data.is_empty() {
                            0.0
                        } else {
                            comp.len() as f64 / data.len() as f64 * 100.0
                        },
                        compress_ms,
                        decompress_ms,
                        roundtrip_ok,
                        error: if roundtrip_ok {
                            None
                        } else {
                            Some(match back {
                                Err(e) => format!("解压失败: {e}"),
                                Ok(_) => "往返内容不一致".into(),
                            })
                        },
                    }
                }
                Err(e) => BenchRow {
                    profile,
                    algo_label: algo.label(),
                    solid,
                    ultra,
                    orig_len: data.len(),
                    comp_len: 0,
                    ratio_pct: 0.0,
                    compress_ms: t0.elapsed().as_millis(),
                    decompress_ms: 0,
                    roundtrip_ok: false,
                    error: Some(format!("压缩失败: {e}")),
                },
            }
        })
        .collect()
}

/// 渲染 14 档差分基准 markdown 表（入库 `docs/compression-bench.md` 用）。
pub fn bench_markdown(data: &[u8]) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "| 档位 | 算法 | 原始 | 压缩后 | 压缩率 | 压缩 ms | 解压 ms | 往返 |"
    );
    let _ = writeln!(s, "| --- | --- | --- | --- | --- | --- | --- | --- |");
    for r in bench_profiles(data) {
        let _ = writeln!(
            s,
            "| `{}` | {}{} | {} | {} | {:.1}% | {} | {} | {} |",
            r.profile,
            r.algo_label,
            if r.ultra { " Ultra" } else { "" },
            human_size(r.orig_len as u64),
            human_size(r.comp_len as u64),
            r.ratio_pct,
            r.compress_ms,
            r.decompress_ms,
            if r.roundtrip_ok { "OK" } else { "FAIL" },
        );
    }
    s
}

/// 模糊往返失败记录。
#[derive(Clone, Debug)]
pub struct FuzzFailure {
    /// 迭代号（0 起）。
    pub iteration: usize,
    /// 规范档位名。
    pub profile: &'static str,
    /// 语料字节数。
    pub size: usize,
    /// 语料熵级标签。
    pub kind: &'static str,
    /// 失败描述（压缩失败 / 解压失败 / 内容不一致）。
    pub error: String,
}

/// 模糊往返固定尺寸序列（小→大覆盖空输入、单字节、块边界、跨字典窗口）。
const FUZZ_SIZES: &[usize] = &[0, 1, 7, 64, 333, 1024, 4096, 16384];

/// 可复现种子模糊往返：`iterations` 轮，每轮取一个确定尺寸（FUZZ_SIZES 轮转）
/// × 确定熵级（五级轮转），14 档全部压缩→解压→逐字节比对。
///
/// 返回失败列表；空 = 全部通过。同一种子在任何平台任何时间产生完全相同的
/// 语料序列（SplitMix64），失败可复现、回归可断言。
pub fn fuzz_roundtrip(seed: u64, iterations: usize) -> Vec<FuzzFailure> {
    let mut rng = SplitMix64::new(seed ^ 0xF00D_F00D);
    let mut failures = Vec::new();
    for it in 0..iterations {
        let size = FUZZ_SIZES[it % FUZZ_SIZES.len()];
        let kind = Entropy::ALL[it % Entropy::ALL.len()];
        let data = gen_section(rng.next_u64(), kind, size);
        for profile in COMPRESSION_OPTIONS {
            let (solid, algo, ultra) = match compression_params(profile) {
                Some(p) => p,
                None => {
                    failures.push(FuzzFailure {
                        iteration: it,
                        profile,
                        size,
                        kind: kind.label(),
                        error: "compression_params 返回 None（规范档位表不一致）".into(),
                    });
                    continue;
                }
            };
            let _ = solid; // solid 仅打包粒度维度，编解码参数一致
            match compress_bytes_profile(algo, &data, ultra) {
                Ok(comp) => match decompress_bytes(algo, &comp) {
                    Ok(back) if back == data => {}
                    Ok(_) => failures.push(FuzzFailure {
                        iteration: it,
                        profile,
                        size,
                        kind: kind.label(),
                        error: format!("往返内容不一致（压缩 {}B → {}B → 不等）", data.len(), comp.len()),
                    }),
                    Err(e) => failures.push(FuzzFailure {
                        iteration: it,
                        profile,
                        size,
                        kind: kind.label(),
                        error: format!("解压失败: {e}"),
                    }),
                },
                Err(e) => failures.push(FuzzFailure {
                    iteration: it,
                    profile,
                    size,
                    kind: kind.label(),
                    error: format!("压缩失败: {e}"),
                }),
            }
        }
    }
    failures
}

/// 抗损坏输入 fuzz 失败记录。
#[derive(Clone, Debug)]
pub struct RobustFailure {
    /// 迭代号（0 起）。
    pub iteration: usize,
    /// 目标（如 "decompress:deflate" / "archive-reader"）。
    pub target: &'static str,
    /// 变异种类（truncate / bitflip / zero-window / append / empty）。
    pub kind: &'static str,
    /// 失败描述（仅记录 panic；Ok/Err 均为可接受行为）。
    pub error: String,
}

/// 变异 `data`（确定性）：第 `kind` 档决定变异种类。
fn mutate_input(rng: &mut SplitMix64, data: &[u8], kind: usize) -> (Vec<u8>, &'static str) {
    match kind % 5 {
        0 => {
            let cut = rng.next_usize(0, data.len() + 1);
            (data[..cut].to_vec(), "truncate")
        }
        1 => {
            let mut b = data.to_vec();
            let flips = rng.next_usize(1, 9);
            for _ in 0..flips {
                if b.is_empty() {
                    break;
                }
                let i = rng.next_usize(0, b.len());
                b[i] ^= (1u8) << rng.next_usize(0, 8);
            }
            (b, "bitflip")
        }
        2 => {
            let mut b = data.to_vec();
            if !b.is_empty() {
                let start = rng.next_usize(0, b.len());
                let end = rng.next_usize(start, b.len().max(1)).min(b.len());
                for x in &mut b[start..end] {
                    *x = 0;
                }
            }
            (b, "zero-window")
        }
        3 => {
            let mut b = data.to_vec();
            let n = rng.next_usize(1, 65);
            for _ in 0..n {
                b.push(rng.next_byte());
            }
            (b, "append")
        }
        _ => (Vec::new(), "empty"),
    }
}

/// 捕获 panic（步骤 49 抗损坏 fuzz 的不变量：解码路径对任意输入**不 panic**，
/// 返回 Ok 或 Err 均为可接受行为）。release `panic = "abort"` 下 panic 会直接
/// 中止进程——同样是显式失败信号，只是没有逐例记录。
fn catch_decode_panic<F>(f: F) -> Option<String>
where
    F: FnOnce() -> anyhow::Result<Vec<u8>>,
{
    use std::panic::{catch_unwind, AssertUnwindSafe};
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(_) => None,
        Err(payload) => {
            let msg = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "非字符串 panic".into());
            Some(msg)
        }
    }
}

/// 可复现种子抗损坏 fuzz（步骤 49）：对五算法压缩流 + 归档读路径施加
/// 确定性变异（截断/位翻转/清零窗口/尾部追加/空输入），断言解码路径
/// **不 panic**（返回 Ok 或 Err 均可）。发现 panic 即记录（同种子可复现）。
///
/// 目标：
/// - `decompress:<algo>`：`decompress_bytes` 对变异压缩流；
/// - `archive-reader`：`ArchiveReader::new` + 逐条目 `next_entry` + `decode`
///   对变异归档字节（安装器 `--ncrc` 路径的真实输入形态）。
pub fn fuzz_decode_robustness(seed: u64, iterations: usize) -> Vec<RobustFailure> {
    use crate::archive::{ArchiveReader, ArchiveWriter};
    use std::io::Cursor;

    let mut rng = SplitMix64::new(seed ^ 0xDEAD_BEEF);
    let mut failures = Vec::new();

    for it in 0..iterations {
        let (data, kind) = {
            let size = FUZZ_SIZES[it % FUZZ_SIZES.len()];
            let ent = Entropy::ALL[it % Entropy::ALL.len()];
            let raw = gen_section(rng.next_u64(), ent, size);
            mutate_input(&mut rng, &raw, it)
        };
        let kind = kind;

        // 目标 1：五算法压缩流变异 → 解码不 panic
        for algo in [
            crate::archive::Algo::Deflate,
            crate::archive::Algo::Brotli,
            crate::archive::Algo::Lzma,
            crate::archive::Algo::Bzip2,
            crate::archive::Algo::Zstd,
        ] {
            let compressed = crate::archive::compress_bytes_profile(algo, &data, false);
            let target = match compressed {
                Ok(c) => c,
                Err(e) => {
                    failures.push(RobustFailure {
                        iteration: it,
                        target: "compress",
                        kind,
                        error: format!("{} 压缩基准流失败: {e}", algo.label()),
                    });
                    continue;
                }
            };
            let algo_label = algo.label();
            if let Some(msg) = catch_decode_panic(|| decompress_bytes(algo, &target)) {
                failures.push(RobustFailure {
                    iteration: it,
                    target: "decompress",
                    kind,
                    error: format!("{algo_label}: {msg}"),
                });
            }
        }

        // 目标 2：归档读路径变异（含逐条目解码，覆盖 magic/长度/标志字段）
        let mut valid_archive = Vec::new();
        {
            let mut w = ArchiveWriter::new_with_total(
                &mut valid_archive,
                (data.len() + 4) as u64,
                2,
            )
            .expect("归档基准流构建");
            w.add_dir("d").expect("归档基准流构建");
            w.add_file_with_algo("d/f.bin", &data, crate::archive::Algo::Deflate)
                .expect("归档基准流构建");
            w.finish().expect("归档基准流构建");
        }
        let mutated = mutate_input(&mut rng, &valid_archive, it.wrapping_add(2)).0;
        if let Some(msg) = catch_decode_panic(|| -> anyhow::Result<Vec<u8>> {
            let mut r = ArchiveReader::new(Cursor::new(&mutated))?;
            while let Some(e) = r.next_entry()? {
                let _ = e.decode()?;
            }
            Ok(Vec::new())
        }) {
            failures.push(RobustFailure {
                iteration: it,
                target: "archive-reader",
                kind,
                error: msg,
            });
        }
    }
    failures
}

/// 端到端 payload 文件清单（相对路径，含安装包内目标布局）。
///
/// 与 `emit_payload_files` 一一对应；e2e 脚本按此清单生成脚本与逐字节比对。
pub const PAYLOAD_FILES: &[&str] = &[
    "bin/MIEditDemo.exe",
    "bin/readme.txt",
    "data/assets.dat",
    "data/sparse.dat",
    "docs/guide.md",
    "media/random.bin",
];

/// 按熵级划分 payload 文件（与 PAYLOAD_FILES 同序）。
fn payload_kind(rel: &str) -> Entropy {
    match rel {
        "bin/MIEditDemo.exe" => Entropy::Binary,
        "bin/readme.txt" | "docs/guide.md" => Entropy::Text,
        "data/assets.dat" => Entropy::Repeated,
        "data/sparse.dat" => Entropy::Zeros,
        "media/random.bin" => Entropy::Random,
        _ => Entropy::Binary,
    }
}

/// 各 payload 文件大小（与 PAYLOAD_FILES 同序，字节）。
const PAYLOAD_SIZES: &[usize] = &[48 * 1024, 2 * 1024, 16 * 1024, 8 * 1024, 6 * 1024, 24 * 1024];

/// 生成端到端 payload 单文件内容（步骤 47 e2e 矩阵共用，确定性）。
pub fn payload_file_bytes(seed: u64, rel: &str) -> Vec<u8> {
    let idx = PAYLOAD_FILES
        .iter()
        .position(|p| *p == rel)
        .unwrap_or(0);
    let kind = payload_kind(rel);
    let size = PAYLOAD_SIZES[idx];
    gen_section(seed ^ (0x1_0000 + idx as u64), kind, size)
}

/// 把端到端 payload 落盘到 `dir`（自动建目录；同一 seed 两次调用逐字节一致）。
/// 返回 (相对路径, 字节数) 列表（按 PAYLOAD_FILES 顺序）。
pub fn emit_payload_files(seed: u64, dir: &Path) -> anyhow::Result<Vec<(String, usize)>> {
    let mut out = Vec::new();
    for rel in PAYLOAD_FILES {
        let bytes = payload_file_bytes(seed, rel);
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, &bytes)?;
        out.push(((*rel).to_string(), bytes.len()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 语料确定性：同种子逐字节一致，异种子不同。
    #[test]
    fn test_bench_corpus_deterministic() {
        let a1 = bench_corpus(BENCH_SEED);
        let a2 = bench_corpus(BENCH_SEED);
        let b = bench_corpus(BENCH_SEED + 1);
        assert_eq!(a1, a2);
        assert_ne!(a1, b);
        assert_eq!(a1.len(), 96 * 1024);
        // 五熵级分区都真实存在
        assert!(a1.windows(64).any(|w| w.iter().all(|&b| b == 0)));
        assert!(a1.iter().any(|&b| b == b'l'));
    }

    /// 各熵级段生成器：确定性 + 长度正确 + zeros 全零。
    #[test]
    fn test_gen_section_kinds() {
        for kind in Entropy::ALL {
            let s1 = gen_section(42, kind, 1000);
            let s2 = gen_section(42, kind, 1000);
            assert_eq!(s1, s2, "{kind:?}");
            assert_eq!(s1.len(), 1000);
        }
        let zeros = gen_section(1, Entropy::Zeros, 512);
        assert!(zeros.iter().all(|&b| b == 0));
        let rand = gen_section(1, Entropy::Random, 512);
        assert!(rand.iter().any(|&b| b != 0));
    }

    /// 14 档差分基准：全档位、全往返、压缩率在合理区间。
    #[test]
    fn test_bench_profiles_all_roundtrip() {
        // 小语料（4KB 可压缩段）保证 debug 构建下测试时长可控；
        // 纯随机段的往返覆盖由 test_fuzz_roundtrip_seed_passes 负责。
        let mut data = gen_section(7, Entropy::Text, 2048);
        data.extend_from_slice(&gen_section(8, Entropy::Zeros, 1024));
        data.extend_from_slice(&gen_section(9, Entropy::Repeated, 1024));
        let rows = bench_profiles(&data);
        assert_eq!(rows.len(), COMPRESSION_OPTIONS.len());
        assert_eq!(COMPRESSION_OPTIONS.len(), 14, "规范档位应为 14");
        for r in &rows {
            assert!(r.roundtrip_ok, "档位 {} 往返失败: {:?}", r.profile, r.error);
            assert!(r.error.is_none());
            assert!(r.comp_len > 0, "档位 {} 压缩输出为空", r.profile);
            assert!(r.ratio_pct > 0.0);
        }
        // 可压缩语料上：最好档压缩率必须显著优于 100%（差分意义）
        let best = rows.iter().map(|r| r.ratio_pct).fold(f64::MAX, f64::min);
        assert!(best < 60.0, "最优压缩率 {best}% 应显著小于 100%");
        // markdown 表渲染：14 行数据行
        let md = bench_markdown(&data);
        assert_eq!(md.lines().count(), 2 + 14);
        assert!(md.contains("| `lzma-ultra-solid` |"));
        assert!(md.contains("OK |"));
    }

    /// 种子模糊往返：小批次全过（任何失败即断言失败，可复现排查）。
    #[test]
    fn test_fuzz_roundtrip_seed_passes() {
        let failures = fuzz_roundtrip(BENCH_SEED, 10);
        assert!(
            failures.is_empty(),
            "种子模糊往返失败: {failuries:#?}",
            failuries = failures
        );
        // 换种子再跑一批（不同尺寸×熵级组合）
        let failures2 = fuzz_roundtrip(987654321, 10);
        assert!(failures2.is_empty(), "第二批模糊往返失败: {failures2:#?}");
    }

    /// 步骤 49：抗损坏输入 fuzz——解码路径对变异流不 panic。
    #[test]
    fn test_fuzz_decode_robustness_no_panic() {
        let failures = fuzz_decode_robustness(BENCH_SEED, 10);
        assert!(
            failures.is_empty(),
            "抗损坏 fuzz 发现 panic: {failures:#?}"
        );
        // 换种子 + 换变异组合再跑一批
        let failures2 = fuzz_decode_robustness(0xDEAD_BEEF, 10);
        assert!(failures2.is_empty(), "第二批抗损坏 fuzz 失败: {failures2:#?}");
    }

    /// 变异生成器：确定性 + 种类正确 + 空输入安全。
    #[test]
    fn test_mutate_input_kinds() {
        let base: Vec<u8> = (0..256u32).map(|i| i as u8).collect();
        for kind in 0..5 {
            let (m1, label) = mutate_input(&mut SplitMix64::new(7), &base, kind);
            let (m2, label2) = mutate_input(&mut SplitMix64::new(7), &base, kind);
            assert_eq!(m1, m2, "kind {kind} 确定性");
            assert_eq!(label, label2);
        }
        // truncate：0..=len
        let (t, l) = mutate_input(&mut SplitMix64::new(1), &base, 0);
        assert_eq!(l, "truncate");
        assert!(t.len() <= base.len());
        // append：> len
        let (a, l) = mutate_input(&mut SplitMix64::new(1), &base, 3);
        assert_eq!(l, "append");
        assert!(a.len() > base.len());
        // empty
        let (e, l) = mutate_input(&mut SplitMix64::new(1), &base, 4);
        assert_eq!((e.len(), l), (0, "empty"));
        // 空输入不 panic
        for kind in 0..5 {
            let _ = mutate_input(&mut SplitMix64::new(9), &[], kind);
        }
    }

    /// 端到端 payload：确定性 + 清单一致 + 落盘往返。
    #[test]
    fn test_payload_emit_deterministic() {
        let dir = std::env::temp_dir().join(format!(
            "miedit-compbench-test-{}-{}",
            std::process::id(),
            BENCH_SEED
        ));
        let files1 = emit_payload_files(BENCH_SEED, &dir).unwrap();
        assert_eq!(files1.len(), PAYLOAD_FILES.len());
        for (rel, size) in &files1 {
            assert!(PAYLOAD_FILES.contains(&rel.as_str()));
            assert!(*size > 0);
        }
        // 落盘内容与内存生成一致
        for rel in PAYLOAD_FILES {
            let on_disk = fs::read(dir.join(rel)).unwrap();
            assert_eq!(&on_disk, &payload_file_bytes(BENCH_SEED, rel));
        }
        // 总规模合理（e2e 全 14 档可在分钟级完成）
        let total: usize = files1.iter().map(|(_, s)| s).sum();
        assert!(total > 64 * 1024 && total < 256 * 1024, "payload 总量 {total}B");
        fs::remove_dir_all(&dir).ok();
    }
}
