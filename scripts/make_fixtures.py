#!/usr/bin/env python3
"""生成 pptx_selector 的测试 fixture。

用法:
    python3 scripts/make_fixtures.py            # 仅生成小型 fixture
    python3 scripts/make_fixtures.py --big      # 额外生成 >500MB 的大文件 fixture

小型 fixture 全部是可被 ZIP/PPTX 正常解析的最小结构，仅 docProps/app.xml 的
<Application> 值不同，用于模拟 PowerPoint / WPS / 未知创建者等场景。
大文件 fixture 由 1 个 STORED 的大 entry 撑到 >500MB，docProps/app.xml 仍然是
正常的 DEFLATE 压缩，用于验证流式读取的 RSS 增量。

脚本幂等：重复执行会覆盖已有 fixture。
"""

from __future__ import annotations

import argparse
import os
import struct
import sys
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
FIXTURES = os.path.join(ROOT, "tests", "fixtures")

BIG_FIXTURE_NAME = "big_512mb.pptx"
BIG_FIXTURE_BYTES = 520 * 1024 * 1024

APP_XML_HEAD = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n'
    '<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties" '
    'xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes">\r\n'
)
APP_XML_TAIL = "</Properties>\r\n"

CONTENT_TYPES = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n'
    '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
    '<Default Extension="xml" ContentType="application/xml"/>'
    '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
    '<Override PartName="/ppt/presentation.xml" '
    'ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>'
    '<Override PartName="/docProps/app.xml" '
    'ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>'
    "</Types>"
)

RELS = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n'
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
    '<Relationship Id="rId1" '
    'Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" '
    'Target="ppt/presentation.xml"/>'
    "</Relationships>"
)

PRESENTATION = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n'
    '<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">'
    "<p:sldIdLst/>"
    "</p:presentation>"
)


def app_xml(application: str | None) -> str:
    """构造 docProps/app.xml；application 为 None 时不写 <Application> 标签。"""
    body = ""
    if application is not None:
        body = f"<Application>{application}</Application>\r\n"
    return APP_XML_HEAD + body + APP_XML_TAIL


def write_pptx(
    path: str,
    *,
    application: str | None = "Microsoft Office PowerPoint",
    app_xml_entry: str = "docProps/app.xml",
    include_app_xml: bool = True,
    app_xml_method: int = zipfile.ZIP_DEFLATED,
) -> None:
    """写出一个最小但结构合法的 PPTX。"""
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as zf:
        zf.writestr("[Content_Types].xml", CONTENT_TYPES)
        zf.writestr("_rels/.rels", RELS)
        zf.writestr("ppt/presentation.xml", PRESENTATION)
        if include_app_xml:
            zf.writestr(app_xml_entry, app_xml(application), compress_type=app_xml_method)


def write_corrupt(path: str) -> None:
    """写出一个不是 ZIP 的文件。"""
    with open(path, "wb") as fp:
        fp.write(b"This is definitely not a ZIP archive.\n" * 8)


def _flip_central_directory_crc(path: str, entry_name: str) -> None:
    """把某个 entry 在中央目录中记录的 CRC32 取反，用于构造 CRC 不匹配的样本。"""
    with open(path, "rb") as fp:
        data = bytearray(fp.read())

    signature = b"PK\x01\x02"
    name = entry_name.encode("utf-8")
    offset = 0
    while True:
        offset = data.find(signature, offset)
        if offset < 0:
            raise RuntimeError(f"central directory entry not found: {entry_name}")
        name_len = struct.unpack_from("<H", data, offset + 28)[0]
        if bytes(data[offset + 46 : offset + 46 + name_len]) == name:
            crc = struct.unpack_from("<I", data, offset + 16)[0]
            struct.pack_into("<I", data, offset + 16, crc ^ 0xFFFFFFFF)
            break
        offset += 4

    with open(path, "wb") as fp:
        fp.write(data)


def write_big(path: str, total_entry_bytes: int) -> None:
    """写出 >500MB 的 fixture：1 个 STORED 大 entry + 正常 DEFLATE 的 app.xml。

    使用 STORE 方法可以精确控制文件体积，同时保证 ZIP 结构完全合法、可被
    任意标准 ZIP 解析器解析；docProps/app.xml 仍为 DEFLATE，探测逻辑必须
    真正走一遍中央目录定位 + 解压流程。
    """
    chunk = b"\x00" * (1 << 20)
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED, allowZip64=False) as zf:
        zf.writestr("docProps/app.xml", app_xml("Microsoft Office PowerPoint"))
        zf.writestr("[Content_Types].xml", CONTENT_TYPES)
        zf.writestr("_rels/.rels", RELS)
        info = zipfile.ZipInfo("ppt/media/placeholder.bin")
        info.compress_type = zipfile.ZIP_STORED
        info.file_size = total_entry_bytes
        with zf.open(info, "w") as fp:
            written = 0
            while written < total_entry_bytes:
                n = min(len(chunk), total_entry_bytes - written)
                fp.write(chunk[:n])
                written += n
        zf.writestr("ppt/presentation.xml", PRESENTATION)


def build_small() -> list[str]:
    os.makedirs(FIXTURES, exist_ok=True)
    made: list[str] = []

    def out(name: str) -> str:
        p = os.path.join(FIXTURES, name)
        made.append(name)
        return p

    write_pptx(out("pptx_powerpoint.pptx"), application="Microsoft Office PowerPoint")
    write_pptx(out("pptx_wps.pptx"), application="WPS Presentation")
    write_pptx(out("pptx_kingsoft.pptx"), application="Kingsoft Office")
    write_pptx(out("pptx_unknown.pptx"), application="LibreOffice Impress")
    write_pptx(out("pptx_priority.pptx"), application="WPS Microsoft")
    write_pptx(out("pptx_no_application.pptx"), application=None)
    write_pptx(out("pptx_entity.pptx"), application="Microsoft&#174; PowerPoint&#174;")
    write_pptx(out("pptx_store_app_xml.pptx"), app_xml_method=zipfile.ZIP_STORED)
    write_pptx(out("pptx_upper_path.pptx"), app_xml_entry="DocProps/App.xml")
    write_pptx(out("pptx_empty_application.pptx"), application="")
    write_pptx(out("no_app_xml.pptx"), include_app_xml=False)

    # 5 MiB 的 <Application> 取值：高度可压缩，磁盘占用很小，用于验证解压上限防护。
    write_pptx(
        out("pptx_huge_application.pptx"),
        application="Microsoft " + ("A" * (5 * 1024 * 1024)),
    )

    write_corrupt(out("corrupt.pptx"))

    crc_bad = out("pptx_bad_crc.pptx")
    write_pptx(crc_bad, application="Microsoft Office PowerPoint")
    _flip_central_directory_crc(crc_bad, "docProps/app.xml")

    return made


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--big", action="store_true", help="额外生成 >500MB 的 fixture")
    parser.add_argument(
        "--big-only", action="store_true", help="只生成 >500MB 的 fixture"
    )
    parser.add_argument(
        "--big-bytes",
        type=int,
        default=BIG_FIXTURE_BYTES,
        help=f"大 entry 的字节数（默认 {BIG_FIXTURE_BYTES}）",
    )
    args = parser.parse_args(argv)

    os.makedirs(FIXTURES, exist_ok=True)

    if not args.big_only:
        made = build_small()
        for name in made:
            size = os.path.getsize(os.path.join(FIXTURES, name))
            print(f"  {name:<28} {size:>10,} B")

    if args.big or args.big_only:
        path = os.path.join(FIXTURES, BIG_FIXTURE_NAME)
        write_big(path, args.big_bytes)
        size = os.path.getsize(path)
        print(f"  {BIG_FIXTURE_NAME:<28} {size:>10,} B")
        if size <= 500 * 1024 * 1024:
            print(f"ERROR: big fixture is only {size} B (<= 500 MiB)", file=sys.stderr)
            return 1

    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
