#!/usr/bin/env bash
# ============================================================================
#  wine_checks.sh - 第六阶段 B：Windows / Wine 集成验证
# ============================================================================
#  Windows 专属逻辑（CreateProcessW、无控制台窗口、图标）无法在 Linux 原生
#  cargo test 中验证，本脚本用 Wine 做集成验证：
#
#    1. PE 子系统 = Windows GUI（无控制台窗口）且 .rsrc 含图标资源
#    2. CreateProcessW 启动外部程序 + 命令行拼接 + Unicode/空格路径原样传递
#    3. 检测为 WPS -> 启动 WPS 替身
#    4. 检测为 PowerPoint -> 启动 PowerPoint 替身
#    5. WPS 不存在 -> 降级 PowerPoint
#    6. 未知创建者 + Fallback=1 -> 兜底启动 PowerPoint
#    7. 目标文件不存在 -> 退出码 2 且仍兜底启动
#
#  替身程序 examples/mock_launcher.rs 会把 argv 的 UTF-16 码元写入
#  `launched_<名字>.txt`，脚本据此精确断言参数传递。
#
#  注意：Wine 无法 100% 等价 Windows；本脚本通过**不代表**已在真实 Windows 上验证。
#
#  用法：bash scripts/wine_checks.sh
# ============================================================================
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TARGET="x86_64-pc-windows-gnu"
EXE="target/$TARGET/release/pptx_selector.exe"
MOCK="target/$TARGET/release/examples/mock_launcher.exe"

export WINEPREFIX="${WINEPREFIX:-$ROOT/.wine-check}"
export WINEDEBUG="${WINEDEBUG:--all}"

PASS=0
FAIL=0
declare -a FAILURES=()

ok()   { echo "  [PASS] $1"; PASS=$((PASS + 1)); }
bad()  { echo "  [FAIL] $1"; FAIL=$((FAIL + 1)); FAILURES+=("$1"); }

check() { # check <描述> <条件命令...>
    local description="$1"; shift
    if "$@" >/dev/null 2>&1; then ok "$description"; else bad "$description"; fi
}

check_contains() { # check_contains <文件> <子串> <描述>
    local file="$1" needle="$2" description="$3"
    if [[ -f "$file" ]] && grep -qF -- "$needle" "$file"; then
        ok "$description"
    else
        bad "$description（在 $file 中未找到: $needle）"
    fi
}

utf16_hex() { # 输出字符串的 UTF-16LE 码元十六进制（大写、空格分隔）
    python3 - "$1" <<'PY'
import sys
raw = sys.argv[1].encode("utf-16-le")
print(" ".join(f"{int.from_bytes(raw[i:i+2], 'little'):04X}" for i in range(0, len(raw), 2)))
PY
}

# ---------------------------------------------------------------------------
# 构建
# ---------------------------------------------------------------------------
echo "==> 构建 Windows 目标与替身程序"
cargo build --release --target "$TARGET" --quiet
cargo build --release --target "$TARGET" --example mock_launcher --quiet

if [[ ! -f "$EXE" || ! -f "$MOCK" ]]; then
    echo "ERROR: 缺少构建产物" >&2
    exit 1
fi

if [[ ! -d "$WINEPREFIX/drive_c" ]]; then
    echo "==> 初始化 Wine prefix: $WINEPREFIX"
    wineboot -u >/dev/null 2>&1 || true
fi

DIR="$WINEPREFIX/drive_c/pptxcheck"
rm -rf "$DIR"
mkdir -p "$DIR"
cp "$EXE"  "$DIR/pptx_selector.exe"
cp "$MOCK" "$DIR/POWERPNT.EXE"
cp "$MOCK" "$DIR/wpp.exe"
cp tests/fixtures/pptx_wps.pptx        "$DIR/wps-deck.pptx"
cp tests/fixtures/pptx_powerpoint.pptx "$DIR/ppt-deck.pptx"
cp tests/fixtures/pptx_unknown.pptx    "$DIR/unknown-deck.pptx"
cp tests/fixtures/pptx_wps.pptx        "$DIR/测试 文稿.pptx"

# ---------------------------------------------------------------------------
# 1. PE 属性：无控制台窗口 + 图标
# ---------------------------------------------------------------------------
echo
echo "== 1. PE 子系统与图标 =="
subsystem="$(x86_64-w64-mingw32-objdump -p "$EXE" | awk '/^Subsystem/ {print $2}')"
if [[ "$subsystem" == "00000002" ]]; then
    ok "PE Subsystem = 2 (Windows GUI)，即无控制台窗口"
else
    bad "PE Subsystem 应为 00000002，实际 $subsystem"
fi
if x86_64-w64-mingw32-objdump -h "$EXE" | grep -q '\.rsrc'; then
    ok "存在 .rsrc 资源节"
else
    bad "缺少 .rsrc 资源节"
fi
resources="$(x86_64-w64-mingw32-objdump -x "$EXE" 2>/dev/null \
    | sed -n '/Resource Directory section/,/Resources start at/p')"
if grep -q 'ID: 0x000003' <<<"$resources" && grep -q 'ID: 0x00000e' <<<"$resources"; then
    ok "资源含 RT_ICON 与 RT_GROUP_ICON（图标已嵌入）"
else
    bad "资源节中未找到 RT_ICON / RT_GROUP_ICON"
fi

# ---------------------------------------------------------------------------
# 运行辅助
# ---------------------------------------------------------------------------
write_ini() { # write_ini <日志名> <PowerPoint路径> <WPS路径> <EnableLog> <Fallback>
    cat > "$DIR/config.ini" <<EOF
[Paths]
PowerPoint=$2
WPS=$3
LogFile=C:\\pptxcheck\\$1

[Options]
EnableLog=$4
Fallback=$5
EOF
}

cleanup_case() {
    rm -f "$DIR"/launched_*.txt "$DIR"/*.log
}

run_case() { # run_case <Windows 目标路径> -> 设置 CASE_EXIT
    set +e
    ( cd "$DIR" && wine ./pptx_selector.exe "$1" >/dev/null 2>&1 )
    CASE_EXIT=$?
    set -e
    # 被启动的替身是异步进程，等它把文件写完
    wineserver -w >/dev/null 2>&1 || true
    for _ in $(seq 1 100); do
        compgen -G "$DIR/launched_*.txt" >/dev/null && break
        sleep 0.1
    done
}

WIN_WPS="C:\\pptxcheck\\wps-deck.pptx"
WIN_PPT="C:\\pptxcheck\\ppt-deck.pptx"
WIN_UNKNOWN="C:\\pptxcheck\\unknown-deck.pptx"
WIN_UNICODE="C:\\pptxcheck\\测试 文稿.pptx"
WIN_MISSING="C:\\pptxcheck\\does-not-exist.pptx"

# ---------------------------------------------------------------------------
# 2. 检测为 WPS + Unicode/空格路径传递
# ---------------------------------------------------------------------------
echo
echo "== 2. 检测为 WPS：启动 WPS 替身并校验 Unicode 路径 =="
cleanup_case
write_ini "case2.log" 'C:\pptxcheck\POWERPNT.EXE' 'C:\pptxcheck\wpp.exe' 1 0
run_case "$WIN_UNICODE"
check "退出码为 0" test "$CASE_EXIT" -eq 0
check "启动了 WPS 替身" test -f "$DIR/launched_wpp.txt"
check "未启动 PowerPoint 替身" test ! -f "$DIR/launched_POWERPNT.txt"
check_contains "$DIR/case2.log" "决策: 使用 WPS 打开" "日志出现 WPS 决策"
check_contains "$DIR/case2.log" "检测到 Application: WPS Presentation" "日志记录检测结果"
check_contains "$DIR/case2.log" "启动进程创建成功" "日志记录启动成功"

if [[ -f "$DIR/launched_wpp.txt" ]]; then
    argv1_readable="$(awk -F'\t' '$1==1 {print $2}' "$DIR/launched_wpp.txt")"
    argv1_hex="$(awk -F'\t' '$1==1 {print $3}' "$DIR/launched_wpp.txt")"
    expected_hex="$(utf16_hex "$WIN_UNICODE")"
    if [[ "$argv1_hex" == "$expected_hex" ]]; then
        ok "命令行 argv[1] 的 UTF-16 码元与预期完全一致"
    else
        bad "argv[1] 码元不匹配: 实际 [$argv1_hex] 期望 [$expected_hex]"
    fi
    check_contains "$DIR/launched_wpp.txt" "测试 文稿.pptx" "argv[1] 含中文与空格且未被拆分（可读形式）"
    echo "         argv[1] = $argv1_readable"
fi

# ---------------------------------------------------------------------------
# 3. 检测为 PowerPoint
# ---------------------------------------------------------------------------
echo
echo "== 3. 检测为 PowerPoint：启动 PowerPoint 替身 =="
cleanup_case
write_ini "case3.log" 'C:\pptxcheck\POWERPNT.EXE' 'C:\pptxcheck\wpp.exe' 1 0
run_case "$WIN_PPT"
check "退出码为 0" test "$CASE_EXIT" -eq 0
check "启动了 PowerPoint 替身" test -f "$DIR/launched_POWERPNT.txt"
check_contains "$DIR/case3.log" "决策: 使用 PowerPoint 打开" "日志出现 PowerPoint 决策"

# ---------------------------------------------------------------------------
# 4. WPS 不存在 -> 降级 PowerPoint
# ---------------------------------------------------------------------------
echo
echo "== 4. WPS 不存在：降级为 PowerPoint =="
cleanup_case
write_ini "case4.log" 'C:\pptxcheck\POWERPNT.EXE' 'C:\pptxcheck\missing-wpp.exe' 1 0
run_case "$WIN_WPS"
check "退出码为 0" test "$CASE_EXIT" -eq 0
check "启动了 PowerPoint 替身" test -f "$DIR/launched_POWERPNT.txt"
check "未启动 WPS 替身" test ! -f "$DIR/launched_wpp.txt"
check_contains "$DIR/case4.log" "WPS存在: 否" "日志记录 WPS 不存在"
check_contains "$DIR/case4.log" "WPS 不存在，降级为 PowerPoint 打开" "日志出现降级决策"

# ---------------------------------------------------------------------------
# 5. 未知创建者 + Fallback=1 -> 兜底 PowerPoint
# ---------------------------------------------------------------------------
echo
echo "== 5. 未知创建者 + Fallback=1：兜底启动 PowerPoint =="
cleanup_case
write_ini "case5.log" 'C:\pptxcheck\POWERPNT.EXE' 'C:\pptxcheck\wpp.exe' 1 1
run_case "$WIN_UNKNOWN"
check "退出码为 0" test "$CASE_EXIT" -eq 0
check "启动了 PowerPoint 替身" test -f "$DIR/launched_POWERPNT.txt"
check_contains "$DIR/case5.log" "检测到 Application: LibreOffice Impress" "日志记录未知创建者"
check_contains "$DIR/case5.log" "未找到可用的首选启动器，尝试兜底" "日志出现兜底路径"
check_contains "$DIR/case5.log" "兜底: 使用 PowerPoint 打开" "日志出现兜底决策"
check_contains "$DIR/case5.log" "兜底启动成功" "日志记录兜底成功"

# ---------------------------------------------------------------------------
# 6. 未知创建者 + Fallback=0 -> 兜底 WPS
# ---------------------------------------------------------------------------
echo
echo "== 6. 未知创建者 + Fallback=0：兜底启动 WPS =="
cleanup_case
write_ini "case6.log" 'C:\pptxcheck\POWERPNT.EXE' 'C:\pptxcheck\wpp.exe' 1 0
run_case "$WIN_UNKNOWN"
check "退出码为 0" test "$CASE_EXIT" -eq 0
check "启动了 WPS 替身" test -f "$DIR/launched_wpp.txt"
check_contains "$DIR/case6.log" "兜底: 使用 WPS 打开" "日志出现 WPS 兜底决策"

# ---------------------------------------------------------------------------
# 7. 目标文件不存在 -> 退出码 2，且仍兜底启动
# ---------------------------------------------------------------------------
echo
echo "== 7. 目标文件不存在：退出码 2 且兜底启动 =="
cleanup_case
write_ini "case7.log" 'C:\pptxcheck\POWERPNT.EXE' 'C:\pptxcheck\wpp.exe' 1 0
run_case "$WIN_MISSING"
check "退出码为 2 (FileNotFound)" test "$CASE_EXIT" -eq 2
check "仍然兜底启动了 WPS 替身" test -f "$DIR/launched_wpp.txt"
check_contains "$DIR/case7.log" "错误: 目标文件不存在，兜底启动" "日志记录文件不存在"
if [[ -f "$DIR/launched_wpp.txt" ]]; then
    argv1_hex="$(awk -F'\t' '$1==1 {print $3}' "$DIR/launched_wpp.txt")"
    expected_hex="$(utf16_hex "$WIN_MISSING")"
    if [[ "$argv1_hex" == "$expected_hex" ]]; then
        ok "兜底启动时 argv[1] 仍是原目标路径"
    else
        bad "兜底 argv[1] 码元不匹配: 实际 [$argv1_hex] 期望 [$expected_hex]"
    fi
fi

# ---------------------------------------------------------------------------
# 8. 日志格式（BOM + CRLF + 时间戳）
# ---------------------------------------------------------------------------
echo
echo "== 8. 日志字节格式 =="
# 使用最后一个用例留下的日志（前面的 cleanup_case 会清掉旧日志）
FORMAT_LOG="$DIR/case7.log"
if [[ ! -f "$FORMAT_LOG" ]]; then
    bad "找不到用于格式校验的日志 $FORMAT_LOG"
else
    check "日志以 UTF-8 BOM 开头" bash -c "head -c 3 '$FORMAT_LOG' | od -An -tx1 | tr -d ' \n' | grep -qi '^efbbbf$'"
    check "日志使用 CRLF 换行" bash -c "grep -qU \$'\r\$' '$FORMAT_LOG'"
    check "日志以 [YYYY-MM-DD HH:MM:SS] 开头" bash -c "sed -n '1p' '$FORMAT_LOG' | LC_ALL=C sed 's/^\xef\xbb\xbf//' | grep -Eq '^\[[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2}\] '"
    check "日志正文含中文（UTF-8 未被转成 ANSI）" grep -qF "========== 程序启动 ==========" "$FORMAT_LOG"
fi

# ---------------------------------------------------------------------------
# 汇总
# ---------------------------------------------------------------------------
echo
echo "================ Wine 验证汇总 ================"
echo "通过: $PASS"
echo "失败: $FAIL"
if [[ $FAIL -gt 0 ]]; then
    echo "失败项:"
    for item in "${FAILURES[@]}"; do echo "  - $item"; done
fi
echo "注意：Wine 不能 100% 等价 Windows；本次结果**未在真实 Windows 上验证**。"
echo "=============================================="

[[ $FAIL -eq 0 ]]
