//! 压缩差分基准与端到端 payload 生成器（路线图步骤 46/47 配套工具）。
//!
//! 用法：
//! ```text
//! # 1) 打印 14 档差分基准 markdown 表（入库 docs/compression-bench.md）
//! cargo run --release -p miedit-core --example compbench
//!
//! # 2) 种子模糊往返（默认种子 20261120，30 轮 × 14 档）
//! cargo run --release -p miedit-core --example compbench -- --fuzz [seed] [iters]
//!
//! # 3) 落盘端到端 payload（步骤 47 e2e-matrix.sh 用，默认种子）
//! cargo run --release -p miedit-core --example compbench -- --emit-payload <dir>
//! ```
//!
//! 全部输出确定性可复现：同一种子在任意平台任意时间逐字节一致。

use miedit_core::compbench;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--fuzz") => {
            let seed: u64 = args
                .get(1)
                .and_then(|s| s.parse().ok())
                .unwrap_or(compbench::BENCH_SEED);
            let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
            println!("种子模糊往返: seed={seed} iterations={iters} profiles=14");
            let t0 = std::time::Instant::now();
            let failures = compbench::fuzz_roundtrip(seed, iters);
            println!(
                "耗时 {:.1}s，失败 {} 项",
                t0.elapsed().as_secs_f64(),
                failures.len()
            );
            for f in &failures {
                println!(
                    "FAIL it={} profile={} size={} kind={}: {}",
                    f.iteration, f.profile, f.size, f.kind, f.error
                );
            }
            if failures.is_empty() {
                println!("OK：全部通过");
            } else {
                std::process::exit(1);
            }
            Ok(())
        }
        Some("--robust") => {
            let seed: u64 = args
                .get(1)
                .and_then(|s| s.parse().ok())
                .unwrap_or(compbench::BENCH_SEED);
            let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(60);
            println!("抗损坏输入 fuzz: seed={seed} iterations={iters}（五算法压缩流 + 归档读路径，不变量=不 panic）");
            let t0 = std::time::Instant::now();
            let failures = compbench::fuzz_decode_robustness(seed, iters);
            println!(
                "耗时 {:.1}s，panic {} 项",
                t0.elapsed().as_secs_f64(),
                failures.len()
            );
            for f in &failures {
                println!(
                    "PANIC it={} target={} kind={}: {}",
                    f.iteration, f.target, f.kind, f.error
                );
            }
            if failures.is_empty() {
                println!("OK：解码路径无 panic");
            } else {
                std::process::exit(1);
            }
            Ok(())
        }
        Some("--emit-payload") => {
            let dir = args
                .get(1)
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from("payload"));
            let files = compbench::emit_payload_files(compbench::BENCH_SEED, &dir)?;
            let total: usize = files.iter().map(|(_, s)| s).sum();
            println!(
                "端到端 payload 已写入 {}（种子 {}）",
                dir.display(),
                compbench::BENCH_SEED
            );
            for (rel, size) in &files {
                println!("  {rel} ({size}B)");
            }
            println!("共 {} 个文件 / {total}B", files.len());
            Ok(())
        }
        _ => {
            let seed = compbench::BENCH_SEED;
            let data = compbench::bench_corpus(seed);
            println!("## 14 档差分基准表");
            println!();
            println!(
                "- 语料种子：`{seed}`（SplitMix64 确定性语料：32KB 文本 + 24KB 二进制 \
                 + 8KB 零段 + 12KB 重复块 + 20KB 随机，共 {}B；平台 {}/{}）",
                data.len(),
                std::env::consts::OS,
                std::env::consts::ARCH
            );
            println!();
            print!("{}", compbench::bench_markdown(&data));
            Ok(())
        }
    }
}
