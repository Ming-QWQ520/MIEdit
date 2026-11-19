#!/usr/bin/env bash
# =============================================================================
# 14 档端到端矩阵验证（路线图步骤 47，健壮性对齐：自解压链路回归）
#
# 流程：mis build（构建安装包）→ 静默安装（--silent --dir=）→ 逐字节比对（cmp）
# 覆盖全部 14 个规范压缩档位（COMPRESSION_OPTIONS，与 miedit-core 对齐）。
#
# 用法：
#   scripts/e2e-matrix.sh                # 完整构建 + 全矩阵
#   scripts/e2e-matrix.sh --no-build     # 复用已有构建产物（调试矩阵用）
#   scripts/e2e-matrix.sh --keep         # 保留工作目录便于排查
#
# 退出码：0 = 14 档全过；1 = 存在失败（明细打印在表格中）。
# payload 由 miedit-core compbench（步骤 46）确定性生成：
#   bin/MIEditDemo.exe  bin/readme.txt  data/assets.dat
#   data/sparse.dat     docs/guide.md   media/random.bin
# （与 crates/miedit-core/src/compbench.rs 的 PAYLOAD_FILES 一一对应，
#   修改清单时两处必须同步。）
# =============================================================================
set -uo pipefail

TIERS=(
  deflate solid
  brotli brotli-solid
  bzip2 bzip2-solid
  lzma lzma-solid lzma-ultra lzma-ultra-solid
  zstd zstd-solid zstd-ultra zstd-ultra-solid
)

PAYLOAD_FILES=(
  bin/MIEditDemo.exe
  bin/readme.txt
  data/assets.dat
  data/sparse.dat
  docs/guide.md
  media/random.bin
)

NO_BUILD=0
KEEP=0
for arg in "$@"; do
  case "$arg" in
    --no-build) NO_BUILD=1 ;;
    --keep) KEEP=1 ;;
    *) echo "未知参数: $arg（支持 --no-build / --keep）"; exit 2 ;;
  esac
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

JOBS="${JOBS:-1}"   # 沙箱内存有限，默认单作业（mzstd 并行编译可能 OOM）

if [ "$NO_BUILD" -eq 0 ]; then
  echo "== 构建 mis / miedit-installer / miedit-uninstaller / compbench (debug, -j $JOBS) =="
  cargo build -p miedit-compiler -p miedit-installer -p miedit-uninstaller -j "$JOBS" || exit 1
  cargo build --release -p miedit-core --example compbench -j "$JOBS" || exit 1
fi

BIN="$ROOT/target/e2e-bin"
mkdir -p "$BIN"
# strip 去符号副本：debug 二进制自带符号可达数百 MB，而 pkg 每档都内嵌整份 stub；
# strip 仅去 debuginfo，运行时行为不变（strip 不可用时回退直接用 target/debug）。
BIN_OK=1
for b in mis miedit-installer miedit-uninstaller; do
  cp -f "target/debug/$b" "$BIN/$b" && strip "$BIN/$b" || BIN_OK=0
  [ "$BIN_OK" -eq 0 ] && break
done
if [ "$BIN_OK" -eq 0 ]; then
  echo "strip 不可用，回退使用 target/debug 原始二进制（磁盘占用较大）"
  BIN="$ROOT/target/debug"
fi

WORK="$ROOT/target/e2e-matrix"
rm -rf "$WORK"; mkdir -p "$WORK"

echo "== 生成确定性 payload（种子 20261120）=="
./target/release/examples/compbench --emit-payload "$WORK/payload" >/dev/null || exit 1

echo "== 逐档构建 → 静默安装 → 逐字节比对 =="
printf '| %-18s | %10s | %-8s | %s |\n' "档位" "包大小" "安装退出码" "逐字节比对"
PASS=0; FAIL=0
for tier in "${TIERS[@]}"; do
  DIR="$WORK/$tier"; mkdir -p "$DIR"
  cat > "$DIR/script.mis" <<EOF
[app]
name = "E2E Matrix"
version = "0.1.0"
publisher = "MIEdit Project"
exe = "MIEditDemo.exe"

[install]
compression = "$tier"

[[files]]
source = "../payload/bin/*"
dest = "bin"

[[files]]
source = "../payload/data/*"
dest = "data"

[[files]]
source = "../payload/docs/*"
dest = "docs"

[[files]]
source = "../payload/media/*"
dest = "media"
EOF
  if ! "$BIN/mis" build "$DIR/script.mis" -o "$DIR/pkg.exe" \
        --stub "$BIN/miedit-installer" --uninstaller "$BIN/miedit-uninstaller" \
        >"$DIR/build.log" 2>&1; then
    printf '| %-18s | %10s | %-8s | %s |\n' "$tier" "-" "BUILD FAIL" "见 $DIR/build.log"
    FAIL=$((FAIL+1)); continue
  fi
  PKG_SIZE=$(wc -c < "$DIR/pkg.exe" | tr -d ' ')

  if ! "$DIR/pkg.exe" --silent --dir="$DIR/inst" >"$DIR/install.log" 2>&1; then
    printf '| %-18s | %10s | %-8s | %s |\n' "$tier" "$PKG_SIZE" "INSTALL FAIL" "见 $DIR/install.log"
    FAIL=$((FAIL+1)); continue
  fi

  MISMATCH=""
  for rel in "${PAYLOAD_FILES[@]}"; do
    if ! cmp -s "$WORK/payload/$rel" "$DIR/inst/$rel"; then
      MISMATCH="$MISMATCH $rel"
    fi
  done
  # 额外断言：卸载清单落盘（Linux 由 uninstall.json 承担）
  if [ ! -f "$DIR/inst/uninstall.json" ]; then
    MISMATCH="$MISMATCH uninstall.json缺失"
  fi
  if [ -z "$MISMATCH" ]; then
    printf '| %-18s | %10s | %-8s | %s |\n' "$tier" "$PKG_SIZE" "0" "OK（6 文件 + 清单）"
    PASS=$((PASS+1))
  else
    printf '| %-18s | %10s | %-8s | %s |\n' "$tier" "$PKG_SIZE" "0" "不一致:$MISMATCH"
    FAIL=$((FAIL+1))
  fi
  # 逐档清理大文件（pkg 内嵌完整 stub；磁盘紧张时避免累积）
  if [ "$KEEP" -eq 0 ]; then
    rm -f "$DIR/pkg.exe" "$DIR/inst/uninstall.json"
    rm -rf "$DIR/inst"
  fi
done

echo "== 结果：$PASS 通过 / $FAIL 失败（共 14 档）=="
if [ "$KEEP" -eq 0 ]; then
  rm -rf "$WORK"
fi
if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
exit 0
