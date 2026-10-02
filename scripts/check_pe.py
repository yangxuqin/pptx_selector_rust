#!/usr/bin/env python3
"""校验 Windows 交叉编译产物是一个合格的 x86-64 GUI 子系统 PE，且嵌入了图标资源。

为什么不用 `file(1)` 判断：不同版本的 file 输出格式并不一致，例如同一个 exe
在 file 5.45 上是 `PE32+ executable (GUI) x86-64, for MS Windows`，在 5.48 上
却是 `PE32+ executable for MS Windows 5.02 (GUI), x86-64 (stripped to external PDB)`。
直接 grep 某个固定串会在 CI 与开发机之间来回误判；这里改为直接读 PE 头，
结论与 file/objdump 的版本无关。

用法：python3 scripts/check_pe.py <pptx_selector.exe>
"""

import struct
import sys
from pathlib import Path

IMAGE_FILE_MACHINE_AMD64 = 0x8664
PE32_MAGIC = 0x10B
PE32PLUS_MAGIC = 0x20B
IMAGE_SUBSYSTEM_WINDOWS_GUI = 2
SECTION_HEADER_SIZE = 40

# Optional Header 中 Subsystem 字段的偏移（PE32 与 PE32+ 都是 68）
OPTIONAL_HEADER_SUBSYSTEM_OFFSET = 68


def fail(message: str) -> None:
    print(f"FAIL: {message}")
    sys.exit(1)


def main() -> None:
    if len(sys.argv) != 2:
        fail("用法: check_pe.py <pptx_selector.exe>")

    path = Path(sys.argv[1])
    if not path.is_file():
        fail(f"文件不存在: {path}")

    data = path.read_bytes()
    if len(data) < 0x40 or data[:2] != b"MZ":
        fail("缺少 DOS 头 'MZ'，不是 Windows PE 文件")

    pe_offset = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe_offset : pe_offset + 4] != b"PE\0\0":
        fail("缺少 PE 签名 'PE\\0\\0'")

    coff = pe_offset + 4
    machine, section_count = struct.unpack_from("<HH", data, coff)
    optional_header_size = struct.unpack_from("<H", data, coff + 16)[0]

    optional = coff + 20
    magic = struct.unpack_from("<H", data, optional)[0]
    subsystem = struct.unpack_from("<H", data, optional + OPTIONAL_HEADER_SUBSYSTEM_OFFSET)[0]

    if machine != IMAGE_FILE_MACHINE_AMD64:
        fail(f"Machine 应为 0x{IMAGE_FILE_MACHINE_AMD64:04X} (x86-64)，实际 0x{machine:04X}")
    if magic not in (PE32_MAGIC, PE32PLUS_MAGIC):
        fail(f"Optional Header Magic 非法: 0x{magic:04X}")
    if subsystem != IMAGE_SUBSYSTEM_WINDOWS_GUI:
        fail(f"Subsystem 应为 {IMAGE_SUBSYSTEM_WINDOWS_GUI} (Windows GUI，无控制台)，实际 {subsystem}")

    # 逐个扫描节表，确认 .rsrc（图标）存在且非空
    sections_offset = optional + optional_header_size
    resource_size = 0
    for index in range(section_count):
        entry = sections_offset + index * SECTION_HEADER_SIZE
        name = data[entry : entry + 8].rstrip(b"\0")
        raw_size = struct.unpack_from("<I", data, entry + 16)[0]
        if name == b".rsrc":
            resource_size = raw_size

    if resource_size <= 0:
        fail("未找到非空的 .rsrc 资源段，图标未嵌入")

    kind = "PE32+" if magic == PE32PLUS_MAGIC else "PE32"
    print(
        f"OK: {kind} / x86-64 / Windows GUI / {section_count} 节 / "
        f".rsrc {resource_size} 字节 / 共 {len(data)} 字节"
    )


if __name__ == "__main__":
    main()
