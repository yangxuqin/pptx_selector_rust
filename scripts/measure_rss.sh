#!/usr/bin/env bash
# ============================================================================
#  measure_rss.sh - 第五阶段：流式 ZIP 性能验证（端到端内存测量）
# ============================================================================
#  在真实 release 二进制上测量"探测操作期间"的进程峰值 RSS，并相对于小文件
#  （≈启动基线）的峰值给出增量。
#
#  验收标准：>500MB PPTX 的 RSS 增量 < 20MB。
#  测试不包含 Wine；不修改探测逻辑，也不绕过 ZIP 解析。
#
#  测量方式说明：本机未安装 GNU /usr/bin/time，因此改用 Python 的
#  `resource.getrusage(RUSAGE_CHILDREN).ru_maxrss`——这正是 `/usr/bin/time -v`
#  内部使用的同一数据源（Linux 上单位为 kB）。每次测量都在**独立的 Python
#  进程**中执行，确保统计的只是那一次子进程运行的峰值。
#
#  用法：bash scripts/measure_rss.sh [大fixture路径]
# ============================================================================
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

BIG_FIXTURE="${1:-tests/fixtures/big_512mb.pptx}"
SMALL_FIXTURE="tests/fixtures/pptx_powerpoint.pptx"
BIN="target/release/pptx_selector"

if [[ ! -f "$BIG_FIXTURE" ]]; then
    echo "ERROR: 缺少大 fixture $BIG_FIXTURE" >&2
    echo "       请先执行: python3 scripts/make_fixtures.py --big" >&2
    exit 1
fi

echo "==> 构建 release 二进制"
cargo build --release --quiet

WORK="$(mktemp -d "${TMPDIR:-/tmp}/pptx-rss-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

cp "$BIN" "$WORK/"
cat > "$WORK/config.ini" <<EOF
[Paths]
PowerPoint=$WORK/POWERPNT.EXE
WPS=$WORK/wpp.exe
LogFile=$WORK/out.log

[Options]
EnableLog=1
Fallback=0
EOF
cp "$SMALL_FIXTURE" "$WORK/small.pptx"
cp "$BIG_FIXTURE" "$WORK/big.pptx"

# 每次都在独立 Python 进程中测量，避免 RUSAGE_CHILDREN 跨运行取最大值。
measure_peak_kb() {
    python3 - "$WORK/pptx_selector" "$1" "$WORK" <<'PY'
import resource
import subprocess
import sys

exe, target, cwd = sys.argv[1], sys.argv[2], sys.argv[3]
subprocess.run([exe, target], cwd=cwd,
               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
print(resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss)
PY
}

BIG_SIZE="$(stat -c%s "$WORK/big.pptx")"
echo "==> 大文件: ${BIG_SIZE} 字节"

echo "==> 基线运行（小文件）"
BASELINE_KB="$(measure_peak_kb small.pptx)"
echo "    基线峰值 RSS  : ${BASELINE_KB} kB"

echo "==> 大文件运行（>500MB）"
BIG_KB="$(measure_peak_kb big.pptx)"
echo "    大文件峰值 RSS: ${BIG_KB} kB"

DELTA_KB=$((BIG_KB - BASELINE_KB))
[[ $DELTA_KB -lt 0 ]] && DELTA_KB=0

echo
echo "================ 内存测量原始数据（端到端） ================"
echo "大文件大小       : ${BIG_SIZE} 字节"
echo "基线峰值 RSS     : ${BASELINE_KB} kB"
echo "大文件峰值 RSS   : ${BIG_KB} kB"
echo "增量             : ${DELTA_KB} kB"
echo "验收门槛         : < 20480 kB (20MB)"
echo "============================================================"

if [[ $DELTA_KB -ge 20480 ]]; then
    echo "FAIL: 内存增量 ${DELTA_KB} kB 超过 20MB 门槛" >&2
    exit 1
fi
echo "PASS: 内存增量 ${DELTA_KB} kB < 20MB"
