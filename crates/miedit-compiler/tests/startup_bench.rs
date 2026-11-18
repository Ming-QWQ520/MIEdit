//! 启动耗时基准（步骤 30「优化编译后 exe 安装包打开速度」的可复现测量）。
//!
//! 构建一个真实安装包（可配置 payload 大小），分别测量：
//! 1. 旧启动路径：read_trailer + 同步 verify_crc + 配置解析；
//! 2. 新启动路径（GUI）：read_trailer + 配置解析（CRC/壁纸已移后台）。
//!
//! 忽略运行（`cargo test --release -p miedit-compiler --test startup_bench -- --ignored --nocapture`），
//! CI 不依赖该测试；数字用于对比优化前后。

use miedit_core::config::{Config, FileSpec};
use miedit_core::trailer;
use std::fs::File;
use std::io::{Seek, Write};
use std::time::Instant;

fn build_test_package(payload_mb: usize, dir: &std::path::Path) -> std::path::PathBuf {
    // 最小配置（一个文件规则即可）
    let mut cfg = Config::default();
    cfg.app.name = "Bench App".into();
    cfg.app.version = "1.0.0".into();
    cfg.files.push(FileSpec {
        source: "bench/data.bin".into(),
        dest: String::new(),
    });
    let config_json = serde_json::to_vec(&cfg).unwrap();

    // payload：可复现的伪随机数据，大小可控
    let payload_len = payload_mb * 1024 * 1024;
    let mut payload = vec![0u8; payload_len];
    for (i, b) in payload.iter_mut().enumerate() {
        *b = (i % 251) as u8;
    }

    let path = dir.join(format!("bench-{}mb.miedit-pkg", payload_mb));
    let mut f = File::create(&path).unwrap();
    let payload_off = 0u64;
    let config_off = payload_off + payload.len() as u64;
    f.write_all(&payload).unwrap();
    f.write_all(&config_json).unwrap();
    let end = config_off + config_json.len() as u64;

    let empty = trailer::Range { off: end, len: 0 };
    let t = trailer::Trailer {
        payload: trailer::Range {
            off: payload_off,
            len: payload.len() as u64,
        },
        config: trailer::Range {
            off: config_off,
            len: config_json.len() as u64,
        },
        uninstaller: empty,
        background: empty,
        icon: empty,
        uninstall_icon: empty,
        uninstall_bg: empty,
        crc: Some(trailer::compute_crc(&payload, &config_json, &[])),
    };
    trailer::write_trailer(&mut f, &t).unwrap();
    f.sync_all().unwrap();
    path
}

fn measure(path: &std::path::Path, with_crc: bool) -> (u128, u128, u128) {
    // 阶段 1：读包尾
    let t0 = Instant::now();
    let tr = trailer::read_trailer(path).unwrap().unwrap();
    let t_trailer = t0.elapsed().as_millis();

    // 阶段 2（可选）：同步 CRC 校验（旧路径关键段）
    let t1 = Instant::now();
    if with_crc {
        trailer::verify_crc(path, &tr).unwrap();
    }
    let t_crc = t1.elapsed().as_millis();

    // 阶段 3：读配置 + 解析
    let t2 = Instant::now();
    let mut f = File::open(path).unwrap();
    let cfg_bytes = trailer::read_range(&mut f, tr.config).unwrap();
    let _cfg: Config = serde_json::from_slice(&cfg_bytes).unwrap();
    drop(f);
    let t_cfg = t2.elapsed().as_millis();
    (t_trailer, t_crc, t_cfg)
}

#[test]
#[ignore]
fn startup_path_bench() {
    let dir = std::env::temp_dir().join("miedit-bench");
    std::fs::create_dir_all(&dir).unwrap();
    for mb in [8usize, 64, 256] {
        let path = build_test_package(mb, &dir);
        // 预热文件缓存后各测 3 次取最小值
        let mut old = (u128::MAX, u128::MAX, u128::MAX);
        let mut new = (u128::MAX, u128::MAX, u128::MAX);
        for _ in 0..3 {
            let m = measure(&path, true);
            if m.0 + m.1 + m.2 < old.0 + old.1 + old.2 {
                old = m;
            }
            let m = measure(&path, false);
            if m.0 + m.2 < new.0 + new.2 {
                new = m;
            }
        }
        println!(
            "[{mb}MB 包] 旧启动关键路径(trailer+同步CRC+config): {:?} ms；新 GUI 路径(trailer+config): {:?} ms（CRC/壁纸后台化）",
            (old.0, old.1 + old.2),
            (new.0, new.2)
        );
        // 确认两路径均正确完成（校验失败会 unwrap panic）
        assert!(old.1 < u128::MAX);
    }
    std::fs::remove_dir_all(&dir).ok();
}
