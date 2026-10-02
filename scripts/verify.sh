#!/usr/bin/env bash
# ============================================================================
#  verify.sh - 一键执行说明书第 11 节的验收清单
# ============================================================================
#  顺序执行并汇总：
#    cargo fmt --check
#    cargo clippy --all-targets -- -D warnings
#    cargo test
#    cargo build --release
#    cargo build --release --target x86_64-pc-windows-gnu
#    内存测量（>500MB，进程内 + 端到端）
#    Wine 集成验证
#    源码约束检查（运行路径禁止 unwrap/expect）
#    原项目零改动检查
#
#  大文件 fixture 默认跳过（很占磁盘）。需要时：
#    python3 scripts/make_fixtures.py --big
#    bash scripts/verify.sh --with-big
# ============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

WITH_BIG=0
[[ "${1:-}" == "--with-big" ]] && WITH_BIG=1

BIG_FIXTURE="tests/fixtures/big_512mb.pptx"
ORIGINAL_REPO="../pptx_selector_next"

STEP_PASS=0
STEP_FAIL=0
declare -a STEP_FAILURES=()

step() { # step <描述> <命令...>
    local description="$1"; shift
    echo
    echo "================================================================"
    echo ">>> $description"
    echo "================================================================"
    if "$@"; then
        echo "--- PASS: $description"
        STEP_PASS=$((STEP_PASS + 1))
    else
        echo "--- FAIL: $description"
        STEP_FAIL=$((STEP_FAIL + 1))
        STEP_FAILURES+=("$description")
    fi
}

step_skip() { echo; echo ">>> SKIP: $1"; }

# 1. 格式与静态检查
step "cargo fmt --check"            cargo fmt --check
step "cargo clippy (Linux, -D warnings)"  cargo clippy --all-targets -- -D warnings
step "cargo clippy (Windows target, -D warnings)" cargo clippy --target x86_64-pc-windows-gnu --all-targets -- -D warnings

# 2. 测试
step "cargo test"                   cargo test

# 3. 构建
step "cargo build --release (Linux 原生)" cargo build --release
step "cargo build --release --target x86_64-pc-windows-gnu" cargo build --release --target x86_64-pc-windows-gnu

# 4. 内存
if [[ -f "$BIG_FIXTURE" ]]; then
    step "内存测量：进程内 >500MB (release)" \
        cargo test --release --config 'profile.release.panic="unwind"' -- --ignored --nocapture memory_peak
    step "内存测量：端到端 /usr/bin/time 等价" bash scripts/measure_rss.sh
else
    step_skip "内存测量（缺少 $BIG_FIXTURE，执行 python3 scripts/make_fixtures.py --big 后重试）"
fi

# 5. Wine 集成验证
step "Wine 集成验证" bash scripts/wine_checks.sh

# 6. 源码约束：运行路径禁止 unwrap()/expect()
echo
echo "================================================================"
echo ">>> 源码约束：src/ 运行路径禁止 .unwrap() / .expect()"
echo "================================================================"
VIOLATIONS="$(grep -rnE '\.unwrap\(\)|\.expect\(' src/ \
    | grep -v 'mod tests' || true)"
# 只统计非测试代码：过滤掉 #[cfg(test)] 之后的行不方便做，这里按"测试模块内允许"人工核对
NON_TEST_VIOLATIONS="$(python3 - <<'PY'
import pathlib
import re

violations = []
for path in sorted(pathlib.Path("src").glob("*.rs")):
    text = path.read_text(encoding="utf-8")
    # 测试模块从 `#[cfg(test)]` 开始；其前的代码不得使用 unwrap/expect
    match = re.search(r"^#\[cfg\(test\)\]\s*$", text, re.M)
    production = text[: match.start()] if match else text
    for number, line in enumerate(production.splitlines(), start=1):
        stripped = line.strip()
        # 注释（含文档注释）里提到这些 API 不算违规
        if stripped.startswith("//"):
            continue
        if re.search(r"\.unwrap\(\)|\.expect\(", line):
            violations.append(f"{path}:{number}: {stripped}")
print("\n".join(violations))
PY
)"
if [[ -z "$NON_TEST_VIOLATIONS" ]]; then
    echo "--- PASS: 生产代码中未出现 .unwrap() / .expect()"
    STEP_PASS=$((STEP_PASS + 1))
else
    echo "$NON_TEST_VIOLATIONS"
    echo "--- FAIL: 生产代码中出现 .unwrap() / .expect()"
    STEP_FAIL=$((STEP_FAIL + 1))
    STEP_FAILURES+=("生产代码禁止 unwrap/expect")
fi
[[ -z "$VIOLATIONS" ]] || echo "（提示：测试代码中的 unwrap/expect 是允许的）"

# 7. 原项目零改动
echo
echo "================================================================"
echo ">>> 原项目零改动检查（$ORIGINAL_REPO）"
echo "================================================================"
if [[ -d "$ORIGINAL_REPO/.git" ]]; then
    DIRTY="$(git -C "$ORIGINAL_REPO" status --porcelain)"
    if [[ -z "$DIRTY" ]]; then
        echo "--- PASS: 原项目工作区干净，未做任何改动"
        STEP_PASS=$((STEP_PASS + 1))
    else
        echo "$DIRTY"
        echo "--- FAIL: 原项目被改动"
        STEP_FAIL=$((STEP_FAIL + 1))
        STEP_FAILURES+=("原项目零改动")
    fi
else
    echo "--- SKIP: 找不到 $ORIGINAL_REPO/.git"
fi

# 8. 交付物清单
echo
echo "================================================================"
echo ">>> 构建产物"
echo "================================================================"
for artifact in target/release/pptx_selector target/x86_64-pc-windows-gnu/release/pptx_selector.exe; do
    if [[ -f "$artifact" ]]; then
        printf '%-52s %10s 字节\n' "$artifact" "$(stat -c%s "$artifact")"
    fi
done

echo
echo "================ 验收汇总 ================"
echo "通过步骤: $STEP_PASS"
echo "失败步骤: $STEP_FAIL"
if [[ $STEP_FAIL -gt 0 ]]; then
    for item in "${STEP_FAILURES[@]}"; do echo "  - $item"; done
fi
echo "=========================================="

[[ $STEP_FAIL -eq 0 ]]
