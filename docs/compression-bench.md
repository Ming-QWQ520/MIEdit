# 压缩差分基准与端到端矩阵（路线图步骤 46/47 入库）

> 本文件是压缩链路的**回归防护基线**：14 档压缩方法在同一确定性语料上的
> 压缩率/耗时差分表 + 可复现种子模糊往返记录 + 14 档端到端矩阵验证结果。
> 任何触碰压缩链路（`miedit_core::archive` / `miedit_core::compbench`）的改动
> 都应重跑下述命令并对比本表，防止格式/性能回归。

## 一、方法论

- **语料**：`compbench::bench_corpus(20261120)`，SplitMix64 确定性生成
  （32KB 文本 + 24KB 二进制 + 8KB 零段 + 12KB 重复块 + 20KB 随机，共 98,304B）。
  同一种子在任意平台任意时间逐字节一致，表格可复现、可对比。
- **测量**：逐档（`COMPRESSION_OPTIONS` 全部 14 个规范档位）压缩 → 解压 →
  逐字节校验；耗时为 release 构建单次运行值（精度 ±1ms，仅作档位间相对参考）。
- **说明**：solid（整包）与孪生条目档编解码参数完全一致，差异只在打包粒度，
  故单流基准两行数值相同；包体差异见第三节 e2e 矩阵的包大小列
  （该列由 stub 体积主导，payload 很小时不敏感）。
- **复现命令**：
  ```bash
  cargo run --release -p miedit-core --example compbench            # 本表
  cargo run --release -p miedit-core --example compbench -- --fuzz [seed] [iters]
  ```

## 二、14 档差分基准表（2026-11-20，linux/x86_64，release）

| 档位 | 算法 | 原始 | 压缩后 | 压缩率 | 压缩 ms | 解压 ms | 往返 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `deflate` | Deflate | 96.0 KB | 40.9 KB | 42.6% | 3 | 0 | OK |
| `solid` | Deflate | 96.0 KB | 40.9 KB | 42.6% | 3 | 0 | OK |
| `brotli` | Brotli | 96.0 KB | 38.8 KB | 40.5% | 19 | 0 | OK |
| `brotli-solid` | Brotli | 96.0 KB | 38.8 KB | 40.5% | 16 | 0 | OK |
| `bzip2` | BZip2 | 96.0 KB | 39.7 KB | 41.3% | 18 | 2 | OK |
| `bzip2-solid` | BZip2 | 96.0 KB | 39.7 KB | 41.3% | 14 | 2 | OK |
| `lzma` | LZMA | 96.0 KB | 38.4 KB | 40.0% | 38 | 2 | OK |
| `lzma-solid` | LZMA | 96.0 KB | 38.4 KB | 40.0% | 30 | 2 | OK |
| `lzma-ultra` | LZMA Ultra | 96.0 KB | 38.4 KB | 40.0% | 49 | 2 | OK |
| `lzma-ultra-solid` | LZMA Ultra | 96.0 KB | 38.4 KB | 40.0% | 45 | 2 | OK |
| `zstd` | Zstd | 96.0 KB | 38.3 KB | 39.8% | 16 | 0 | OK |
| `zstd-solid` | Zstd | 96.0 KB | 38.3 KB | 39.8% | 15 | 0 | OK |
| `zstd-ultra` | Zstd Ultra | 96.0 KB | 38.3 KB | 39.8% | 15 | 0 | OK |
| `zstd-ultra-solid` | Zstd Ultra | 96.0 KB | 38.3 KB | 39.8% | 15 | 0 | OK |

**读表要点**：
- 96KB 语料上 zstd / lzma / brotli 三强档差距在 ±0.7% 内（语料含 20KB 纯随机
  下界，天花板受限）；**大语料下的排序见 #22 记录**（2.86MB 混合语料：
  lzma 9.8% 最小、zstd 15.5% 解压最快、bzip2 15.3%、deflate 21.8%）。
- **ultra 档与标准档在小语料上持平**（LZMA 64MB 字典 / zstd level 22 的收益
  与语料规模正相关）——诚实记录，与 #22 结论一致；选档依据是包体量：
  MB 级以下选标准档即可，ultra 面向几十 MB 以上的安装体量。
- 解压耗时排序：zstd / brotli / deflate ≈ 0ms < bzip2 / lzma ≈ 2ms，
  与 #22 的 2.86MB 实测一致（zstd 解压最快优势在语料放大后更明显）。

## 三、种子模糊往返（fuzz roundtrip）

`compbench::fuzz_roundtrip`：SplitMix64 驱动，固定尺寸序列
（0 / 1 / 7 / 64 / 333 / 1024 / 4096 / 16384 B 轮转）× 五熵级轮转
（text / binary / zeros / repeated / random），14 档全部压缩 → 解压 →
逐字节比对，失败收集为 `FuzzFailure`（含档位/尺寸/熵级/原因，同种子可复现）。

本轮记录（release，双种子各 60 轮）：

| 种子 | 轮数 × 档位 | 往返总数 | 失败 |
| --- | --- | --- | --- |
| 20261120 | 60 × 14 | 840 | **0** |
| 987654321 | 60 × 14 | 840 | **0** |

单测防护（`cargo test -p miedit-core compbench`）：
`test_fuzz_roundtrip_seed_passes` 双种子小批次断言全过；
`test_bench_profiles_all_roundtrip` 14 档语料往返 + 表渲染断言；
`test_payload_emit_deterministic` 端到端 payload 确定性断言。

## 四、14 档端到端矩阵验证（步骤 47 + 步骤 48 卸载扩展）

`scripts/e2e-matrix.sh`：确定性 payload（`compbench::emit_payload_files`，
六文件约 104KB：文本 ×2 / 二进制 / 重复块 / 零段 / 纯随机）→ 逐档
`mis build`（真实编译链，含许可/清单/Trailer 全流程）→ 安装器
`--silent --dir=` 静默安装（退出码 0）→ 六文件 `cmp` 逐字节比对 +
`uninstall.json` 清单落盘断言 → **静默卸载**（`--silent`，退出码 0）→
**全清断言**（六文件 / 清单 / 卸载器本体 / 安装目录全部移除）。

本轮记录（2026-11-20，Linux 无头，debug 构建，**安装比对 + 卸载清理 14/14 全过**）：

| 档位 | 包大小* | 安装退出码 | 逐字节比对 |
| --- | --- | --- | --- |
| deflate / solid | 75,379,793 / 75,380,625 | 0 | OK（6 文件 + 清单） |
| brotli / brotli-solid | 75,379,858 / 75,378,267 | 0 | OK（6 文件 + 清单） |
| bzip2 / bzip2-solid | 75,380,275 / 75,381,598 | 0 | OK（6 文件 + 清单） |
| lzma / lzma-solid | 75,377,856 / 75,377,871 | 0 | OK（6 文件 + 清单） |
| lzma-ultra / lzma-ultra-solid | 75,377,862 / 75,377,877 | 0 | OK（6 文件 + 清单） |
| zstd / zstd-solid | 75,377,289 / 75,377,306 | 0 | OK（6 文件 + 清单） |
| zstd-ultra / zstd-ultra-solid | 75,377,295 / 75,377,312 | 0 | OK（6 文件 + 清单） |

\* 调试构建 stub（约 71MB）体积主导包体，payload 压缩差异不敏感；
大小差异的权威数据见第二节差分基准表与 #22 的 2.86MB 实测。

**顺带修复**（e2e 发现的真实缺陷）：
- #23：`mis build` 在 Linux 上 `File::create` 新建输出文件丢失 stub 的
  可执行位（0644），成品包无法直接运行；已在编译器写出后回授 stub 权限位
  （`#[cfg(unix)]`，Windows 不受影响）。
- #24（卸载扩展阶段发现）：① Linux 清单 `manifest_path` 在序列化后才回填，
  清单文件内恒为空串 → 卸载器永远删不掉 `uninstall.json`；改为序列化前
  回填。② 卸载器 cleanup 兜底（空路径回退 exe 同目录 uninstall.json）
  + 清理安装器副产物 `install.log`。③ `schedule_self_delete` Linux 分支
  此前 no-op（本体与目录永不清理）→ 后台 `sh -c` 自删除落地。修复前
  14/14 档目录残留（uninstall.json + install.log），修复后 14/14 全清。

## 四.1、抗损坏输入 fuzz（步骤 49）

`compbench::fuzz_decode_robustness`：对五算法压缩流与归档读路径
（`ArchiveReader::new` + 逐条目 `next_entry` + `decode`，即安装器
`--ncrc` 路径的真实输入形态）施加五种确定性变异（截断/位翻转/清零
窗口/尾部追加/空输入），不变量 = **解码路径不 panic**（Ok/Err 均可接受）。

本轮记录（release，三种子各 120 轮 × 6 目标 = 2160 变异案例）：

| 种子 | 轮数 | 变异案例 | panic |
| --- | --- | --- | --- |
| 20261120 | 120 | 720 | **0** |
| 987654321 | 120 | 720 | **0** |
| 3735928559 | 120 | 720 | **0** |

配套加固：`MAX_ENTRY_BYTES`（2 GiB）条目长度上限——损坏归档的
comp_len/orig_len 遇 u64::MAX 级字段此前直接 OOM abort，现优雅报错
（单测覆盖 u64::MAX 与 2 GiB+1 两档拒绝路径）。
复现命令：`cargo run --release -p miedit-core --example compbench -- --robust [seed] [iters]`。

## 五、回归防护清单

改压缩链路后按序重跑：

```bash
# 1) core 单测（含 compbench 7 项 + 压缩档映射/往返既有测试）
cargo test -p miedit-core
# 2) 重跑差分基准并对比第二节表格（release）
cargo run --release -p miedit-core --example compbench
# 3) 深模糊（双种子 60 轮应为 0 失败）
cargo run --release -p miedit-core --example compbench -- --fuzz 20261120 60
cargo run --release -p miedit-core --example compbench -- --fuzz 987654321 60
# 4) 抗损坏 fuzz（三种子 0 panic）
cargo run --release -p miedit-core --example compbench -- --robust 20261120 120
cargo run --release -p miedit-core --example compbench -- --robust 987654321 120
cargo run --release -p miedit-core --example compbench -- --robust 3735928559 120
# 5) 端到端矩阵（14/14 应全过，含安装比对 + 卸载清理断言）
scripts/e2e-matrix.sh
```
