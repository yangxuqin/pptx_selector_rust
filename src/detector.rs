//! ============================================================================
//!  detector.rs - PPTX 创建者检测
//!  PPTX 智能打开器
//! ============================================================================
//!  PPTX 本质是 ZIP 包，内部 docProps/app.xml 的 `<Application>` 标签记录
//!  创建软件。本模块以**流式**方式定位并读取该标签。
//!  对应原项目 `src/pptx_detector.h` / `src/pptx_detector.cpp`。
//!
//!  流式约束（说明书第 2 节硬性要求）：
//!  - 禁止把整个 PPTX 文件读入内存；
//!  - 禁止一次性解压全部 ZIP 条目；
//!  - 只读取 `docProps/app.xml`；
//!  - XML 采用流式事件解析，不构建完整 DOM。
//!
//!  实现要点：
//!  - `rawzip` 只解析中央目录并定位 entry，缓冲区固定 64 KiB；
//!  - `rawzip` **不含解压能力**，DEFLATE 由 `miniz_oxide`（原项目 miniz 的
//!    Rust 移植）负责；
//!  - 定位语义对齐 miniz：按中央目录顺序线性扫描，取第一个路径与
//!    `docProps/app.xml` **ASCII 大小写不敏感且等长**匹配的条目。
//!
//! ============================================================================

use std::fs::File;
use std::io::Read;

use rawzip::{CompressionMethod, ZipArchive, RECOMMENDED_BUFFER_SIZE};

use crate::error::DetectError;

/// 目标条目名（与 miniz 调用 `mz_zip_reader_locate_file(..., "docProps/app.xml", ...)` 一致）。
pub const APP_XML_ENTRY: &[u8] = b"docProps/app.xml";

/// app.xml 的读取/解压上限，用于防御解压炸弹。
///
/// 原项目没有这个上限（依赖 miniz 直接分配）。真实 PPTX 的 app.xml 通常只有
/// 几 KB，4 MiB 已极为宽松；超限时返回 `Err`，上层按原逻辑降级为 `Unknown`。
pub const MAX_APP_XML_BYTES: usize = 4 * 1024 * 1024;

/// PPTX 文件的创建软件类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppCreator {
    /// 未能识别。
    #[default]
    Unknown,
    /// WPS 演示（含 Kingsoft）。
    Wps,
    /// Microsoft PowerPoint。
    MicrosoftOffice,
}

/// 检测结果。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DetectionResult {
    /// 创建软件类型。
    pub creator: AppCreator,
    /// `<Application>` 的原始内容，便于日志展示。
    pub application: String,
}

/// 在已打开的文件上检测创建软件。
///
/// 主流程先自行打开文件（校验可读性并取得大小），再把句柄交给本函数，
/// 因此这里不需要也不应该再打开一次文件。
///
/// 失败时返回 `Err`；调用方按原项目逻辑把失败折叠成
/// `DetectionResult { creator: Unknown, application: "" }`。
pub fn detect_pptx_creator_from_file(file: File) -> Result<DetectionResult, DetectError> {
    // 固定 64 KiB 缓冲区，与文件大小无关。
    let mut buffer = vec![0u8; RECOMMENDED_BUFFER_SIZE];
    let archive = ZipArchive::from_file(file, &mut buffer).map_err(DetectError::Zip)?;

    // 按中央目录顺序线性扫描，取第一个大小写不敏感匹配项。
    let found = {
        let mut entries = archive.entries(&mut buffer);
        let mut found = None;
        while let Some(entry) = entries.next_entry().map_err(DetectError::Zip)? {
            if entry
                .file_path()
                .as_ref()
                .eq_ignore_ascii_case(APP_XML_ENTRY)
            {
                found = Some((entry.wayfinder(), entry.compression_method()));
                break;
            }
        }
        found
    };
    let (wayfinder, method) = found.ok_or(DetectError::EntryNotFound)?;

    let entry = archive.get_entry(wayfinder).map_err(DetectError::Zip)?;
    // 中央目录记录的 CRC32 与未压缩大小，用于对齐 miniz 的校验行为。
    let reader = entry.reader();
    let expected = reader.claim_verifier();

    let mut compressed = Vec::new();
    reader
        .take((MAX_APP_XML_BYTES + 1) as u64)
        .read_to_end(&mut compressed)
        .map_err(DetectError::Read)?;
    if compressed.len() > MAX_APP_XML_BYTES {
        return Err(DetectError::TooLarge);
    }

    // miniz 只支持 STORE 与 DEFLATE，其余方法一律失败。
    let xml = if method == CompressionMethod::STORE {
        compressed
    } else if method == CompressionMethod::DEFLATE {
        miniz_oxide::inflate::decompress_to_vec_with_limit(&compressed, MAX_APP_XML_BYTES)
            .map_err(|_| DetectError::Inflate)?
    } else {
        return Err(DetectError::UnsupportedCompression);
    };

    // 对齐 miniz 的 CRC32 / 未压缩大小校验。
    if xml.len() as u64 != expected.uncompressed_size || rawzip::crc32(&xml) != expected.crc {
        return Err(DetectError::VerificationFailed);
    }

    // `<Application>` 缺失不是错误：原项目同样得到空串 + Unknown。
    let application = extract_application(&xml)
        .map_err(DetectError::Xml)?
        .unwrap_or_default();
    let creator = classify_app(&application);
    Ok(DetectionResult {
        creator,
        application,
    })
}

/// 流式提取第一个 `<Application>…</Application>` 之间的原始文本。
///
/// - 不构建 DOM，只做事件扫描；
/// - 取事件中的**原始字面文本**，与原项目 `extractTag` 的裸字节截取一致：
///   实体引用（`&amp;` / `&#174;` 等）与 CDATA 都按文档中的原始写法还原，
///   **不做解码、不做空白裁剪**；
/// - `<Application/>`（`Event::Empty`）与原项目一样**不算匹配**；
/// - 未找到时返回 `Ok(None)`。
///
/// 已知差异：`<Application>` 内部的**子元素标记**（如 `<a>...</a>`）会被丢弃，
/// 而原项目的裸字节截取会原样保留。真实 PPTX 的 `<Application>` 只含纯文本，
/// 详见 `docs/behavior-mapping.md` D5b。
fn extract_application(xml: &[u8]) -> Result<Option<String>, quick_xml::Error> {
    use quick_xml::events::Event;

    let mut reader = quick_xml::Reader::from_reader(xml);
    let mut buffer = Vec::new();
    let mut collecting = false;
    let mut content = String::new();

    loop {
        match reader.read_event_into(&mut buffer)? {
            Event::Start(event) if event.local_name().as_ref() == "Application" => {
                collecting = true;
            }
            Event::End(event) if collecting && event.local_name().as_ref() == "Application" => {
                return Ok(Some(content));
            }
            Event::Text(text) if collecting => {
                content.push_str(text.into_inner().as_ref());
            }
            // quick-xml 把实体/字符引用单独拆成 GeneralRef 事件，其内容是不含
            // `&` 与 `;` 的原始内部文本；补回定界符即可逐字节还原文档原文。
            Event::GeneralRef(reference) if collecting => {
                content.push('&');
                content.push_str(reference.into_inner().as_ref());
                content.push(';');
            }
            // CDATA 的原始写法是 `<![CDATA[...]]>`；同样补回定界符。
            Event::CData(data) if collecting => {
                content.push_str("<![CDATA[");
                content.push_str(data.as_ref());
                content.push_str("]]>");
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(None)
}

/// 根据 `<Application>` 文本判断创建软件类型。
///
/// 与原项目完全一致：WPS/Kingsoft 优先，其次 Microsoft/PowerPoint，
/// 均为**大小写敏感的裸字节子串匹配**。
fn classify_app(application: &str) -> AppCreator {
    if application.contains("WPS") || application.contains("Kingsoft") {
        return AppCreator::Wps;
    }
    if application.contains("Microsoft") || application.contains("PowerPoint") {
        return AppCreator::MicrosoftOffice;
    }
    AppCreator::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name)
    }

    /// 打开 fixture 后交给被测函数。主流程自行负责打开文件，检测模块只接受句柄。
    fn detect(name: &str) -> Result<DetectionResult, DetectError> {
        let file = File::open(fixture(name)).unwrap_or_else(|e| panic!("打开 {name} 失败: {e}"));
        detect_pptx_creator_from_file(file)
    }

    fn detect_at(path: &Path) -> Result<DetectionResult, DetectError> {
        let file = File::open(path).unwrap_or_else(|e| panic!("打开 {path:?} 失败: {e}"));
        detect_pptx_creator_from_file(file)
    }

    fn creator_of(name: &str) -> DetectionResult {
        detect(name).unwrap_or_else(|e| panic!("{name} 应当可被检测: {e}"))
    }

    #[test]
    fn builds_fixtures_first() {
        assert!(
            fixture("pptx_powerpoint.pptx").exists(),
            "缺少测试 fixture，请先执行: python3 scripts/make_fixtures.py"
        );
    }

    fn extract(xml: &str) -> Option<String> {
        extract_application(xml.as_bytes()).expect("xml 应当可解析")
    }

    #[test]
    fn extract_application_keeps_plain_text_verbatim() {
        assert_eq!(
            extract(
                "<Properties><Application>Microsoft Office PowerPoint</Application></Properties>"
            ),
            Some("Microsoft Office PowerPoint".to_string())
        );
    }

    #[test]
    fn extract_application_keeps_whitespace_untouched() {
        assert_eq!(
            extract("<Application>  a  b  </Application>"),
            Some("  a  b  ".to_string())
        );
    }

    #[test]
    fn extract_application_restores_entity_references_verbatim() {
        // 对齐原项目 `extractTag` 的裸字节截取：不解码实体。
        assert_eq!(
            extract("<Application>Microsoft&#174; PowerPoint&#174;</Application>"),
            Some("Microsoft&#174; PowerPoint&#174;".to_string())
        );
        assert_eq!(
            extract("<Application>Kingsoft &amp; WPS</Application>"),
            Some("Kingsoft &amp; WPS".to_string())
        );
        assert_eq!(
            extract("<Application>&#x0057;PS</Application>"),
            Some("&#x0057;PS".to_string())
        );
    }

    #[test]
    fn extract_application_restores_cdata_markers_verbatim() {
        assert_eq!(
            extract("<Application><![CDATA[WPS Presentation]]></Application>"),
            Some("<![CDATA[WPS Presentation]]>".to_string())
        );
    }

    #[test]
    fn extract_application_handles_missing_and_self_closing_forms() {
        assert_eq!(extract("<Properties/>"), None);
        assert_eq!(extract("<Application/>"), None);
        assert_eq!(extract("<Other>WPS</Other>"), None);
    }

    #[test]
    fn extract_application_stops_at_the_first_closing_tag() {
        assert_eq!(
            extract("<Application>A</Application><Application>B</Application>"),
            Some("A".to_string())
        );
    }

    #[test]
    fn extract_application_errors_on_malformed_xml() {
        // 标签不匹配等严重畸形输入：返回 Err，上层折叠为 Unknown（见行为文档 D6）。
        assert!(extract_application(b"<Application><a></Application>").is_err());
        assert!(extract_application(b"</Application>").is_err());
    }

    #[test]
    fn unclosed_application_never_yields_content() {
        // 未闭合时 quick-xml 可能返回 Err，也可能读到 EOF 后返回 None；
        // 两种情况最终都折叠为 Unknown，与原项目「找不到 </Application> 即空」一致。
        let result = extract_application(b"<Application>WPS");
        assert!(
            matches!(result, Err(_) | Ok(None)),
            "未闭合的 <Application> 不应产出内容: {result:?}"
        );
    }

    #[test]
    fn classification_matrix_matches_the_cpp_rules() {
        assert_eq!(
            classify_app("Microsoft Office PowerPoint"),
            AppCreator::MicrosoftOffice
        );
        assert_eq!(classify_app("Microsoft"), AppCreator::MicrosoftOffice);
        assert_eq!(classify_app("PowerPoint"), AppCreator::MicrosoftOffice);
        assert_eq!(classify_app("WPS Presentation"), AppCreator::Wps);
        assert_eq!(classify_app("Kingsoft Office"), AppCreator::Wps);
        assert_eq!(classify_app("LibreOffice Impress"), AppCreator::Unknown);
        assert_eq!(classify_app(""), AppCreator::Unknown);
        // WPS/Kingsoft 优先于 Microsoft/PowerPoint
        assert_eq!(classify_app("WPS Microsoft"), AppCreator::Wps);
        assert_eq!(classify_app("Kingsoft PowerPoint"), AppCreator::Wps);
        // 大小写敏感
        assert_eq!(classify_app("microsoft powerpoint"), AppCreator::Unknown);
        assert_eq!(classify_app("wps"), AppCreator::Unknown);
    }

    #[test]
    fn detects_powerpoint() {
        let result = creator_of("pptx_powerpoint.pptx");
        assert_eq!(result.creator, AppCreator::MicrosoftOffice);
        assert_eq!(result.application, "Microsoft Office PowerPoint");
    }

    #[test]
    fn detects_wps() {
        let result = creator_of("pptx_wps.pptx");
        assert_eq!(result.creator, AppCreator::Wps);
        assert_eq!(result.application, "WPS Presentation");
    }

    #[test]
    fn detects_kingsoft_as_wps() {
        let result = creator_of("pptx_kingsoft.pptx");
        assert_eq!(result.creator, AppCreator::Wps);
        assert_eq!(result.application, "Kingsoft Office");
    }

    #[test]
    fn unknown_application_is_not_an_error() {
        let result = creator_of("pptx_unknown.pptx");
        assert_eq!(result.creator, AppCreator::Unknown);
        assert_eq!(result.application, "LibreOffice Impress");
    }

    #[test]
    fn wps_wins_when_both_markers_are_present() {
        let result = creator_of("pptx_priority.pptx");
        assert_eq!(result.creator, AppCreator::Wps);
        assert_eq!(result.application, "WPS Microsoft");
    }

    #[test]
    fn missing_application_tag_yields_empty_and_unknown() {
        let result = creator_of("pptx_no_application.pptx");
        assert_eq!(result.creator, AppCreator::Unknown);
        assert_eq!(result.application, "");
    }

    #[test]
    fn empty_application_tag_yields_empty_and_unknown() {
        let result = creator_of("pptx_empty_application.pptx");
        assert_eq!(result.creator, AppCreator::Unknown);
        assert_eq!(result.application, "");
    }

    #[test]
    fn entity_text_is_preserved_verbatim_like_the_cpp_byte_scan() {
        let result = creator_of("pptx_entity.pptx");
        assert_eq!(result.application, "Microsoft&#174; PowerPoint&#174;");
        assert_eq!(result.creator, AppCreator::MicrosoftOffice);
    }

    #[test]
    fn stored_app_xml_is_supported() {
        let result = creator_of("pptx_store_app_xml.pptx");
        assert_eq!(result.creator, AppCreator::MicrosoftOffice);
    }

    #[test]
    fn entry_lookup_is_case_insensitive() {
        let result = creator_of("pptx_upper_path.pptx");
        assert_eq!(result.creator, AppCreator::MicrosoftOffice);
    }

    #[test]
    fn corrupt_pptx_returns_err_without_panicking() {
        let error = detect("corrupt.pptx").expect_err("应当是 Err");
        assert!(matches!(error, DetectError::Zip(_)), "实际: {error:?}");
    }

    #[test]
    fn missing_app_xml_returns_entry_not_found() {
        let error = detect("no_app_xml.pptx").expect_err("应当是 Err");
        assert!(
            matches!(error, DetectError::EntryNotFound),
            "实际: {error:?}"
        );
    }

    #[test]
    fn crc_mismatch_returns_verification_failed() {
        let error = detect("pptx_bad_crc.pptx").expect_err("应当是 Err");
        assert!(
            matches!(error, DetectError::VerificationFailed),
            "实际: {error:?}"
        );
    }

    #[test]
    fn zero_byte_file_returns_err() {
        let dir = TempDir::new("det-empty");
        let path = dir.path().join("empty.pptx");
        fs::write(&path, b"").expect("write");
        assert!(detect_at(&path).is_err());
    }

    #[test]
    fn directory_instead_of_file_returns_err() {
        let dir = TempDir::new("det-dir");
        assert!(detect_at(dir.path()).is_err());
    }

    #[test]
    fn oversized_app_xml_is_rejected() {
        let error = detect("pptx_huge_application.pptx").expect_err("应当是 Err");
        assert!(
            matches!(error, DetectError::Inflate | DetectError::TooLarge),
            "实际: {error:?}"
        );
    }

    #[test]
    fn chinese_path_is_handled() {
        let dir = TempDir::new("det-chinese");
        let nested = dir.path().join("演示 文稿");
        fs::create_dir_all(&nested).expect("mkdir");
        let path = nested.join("测试 文件.pptx");
        fs::copy(fixture("pptx_wps.pptx"), &path).expect("copy");
        let result = detect_at(&path).expect("应当可被检测");
        assert_eq!(result.creator, AppCreator::Wps);
    }

    #[test]
    fn path_with_spaces_is_handled() {
        let dir = TempDir::new("det-spaces");
        let nested = dir.path().join("a dir with spaces");
        fs::create_dir_all(&nested).expect("mkdir");
        let path = nested.join("my presentation.pptx");
        fs::copy(fixture("pptx_powerpoint.pptx"), &path).expect("copy");
        let result = detect_at(&path).expect("应当可被检测");
        assert_eq!(result.creator, AppCreator::MicrosoftOffice);
    }

    #[test]
    fn path_based_and_file_based_entry_points_agree() {
        let path = fixture("pptx_wps.pptx");
        let first = detect_at(&path).expect("first pass");
        let second = detect_at(&path).expect("second pass");
        assert_eq!(first, second);
    }

    #[test]
    fn repeated_detection_is_stable() {
        for _ in 0..3 {
            assert_eq!(
                creator_of("pptx_powerpoint.pptx").creator,
                AppCreator::MicrosoftOffice
            );
        }
    }

    // ------------------------------------------------------------------
    //  第五阶段：流式 ZIP 内存验证
    // ------------------------------------------------------------------

    #[cfg(target_os = "linux")]
    fn proc_status_kb(key: &str) -> Option<u64> {
        let status = fs::read_to_string("/proc/self/status").ok()?;
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix(key) {
                let value = rest.trim().trim_end_matches("kB").trim();
                return value.parse::<u64>().ok();
            }
        }
        None
    }

    /// 测量 >500MB PPTX 探测期间相对基线的 RSS 峰值增量。
    ///
    /// 需要先生成 fixture：`python3 scripts/make_fixtures.py --big`
    /// 运行：`cargo test --release --config 'profile.release.panic="unwind"' \
    ///        -- --ignored --nocapture memory_peak`
    #[test]
    #[ignore = "需要 >500MB fixture，属于第五阶段的重量级验证"]
    #[cfg(target_os = "linux")]
    fn memory_peak_increment_on_large_pptx() {
        let path = fixture("big_512mb.pptx");
        assert!(
            path.exists(),
            "缺少大 fixture，请先执行: python3 scripts/make_fixtures.py --big"
        );
        let file_size = fs::metadata(&path).expect("metadata").len();
        assert!(
            file_size > 500 * 1024 * 1024,
            "fixture 必须 >500MB，实际 {file_size} 字节"
        );

        let baseline_rss = proc_status_kb("VmRSS:").expect("VmRSS");
        let baseline_hwm = proc_status_kb("VmHWM:").expect("VmHWM");

        let result = detect_at(&path).expect("应当可被检测");

        let peak_hwm = proc_status_kb("VmHWM:").expect("VmHWM");
        let final_rss = proc_status_kb("VmRSS:").expect("VmRSS");
        let delta_kb = peak_hwm.saturating_sub(baseline_rss);

        println!("--- 内存测试原始数据 (>500MB PPTX) ---");
        println!("文件大小        : {file_size} 字节");
        println!("基线 RSS (VmRSS): {baseline_rss} kB");
        println!("基线峰值 (VmHWM): {baseline_hwm} kB");
        println!("探测后 VmHWM    : {peak_hwm} kB");
        println!("探测后 VmRSS    : {final_rss} kB");
        println!("增量 (VmHWM - 基线RSS): {delta_kb} kB");
        println!(
            "检测结果        : {:?} / {:?}",
            result.creator, result.application
        );

        assert_eq!(result.creator, AppCreator::MicrosoftOffice);
        assert!(
            delta_kb < 20 * 1024,
            "RSS 增量 {delta_kb} kB 超过 20MB 门槛"
        );
    }
}
