//! Loại THẬT của nội dung, đọc từ chữ ký ở đầu file ("magic bytes"), và kích thước ảnh.
//!
//! Client khai loại bằng `Content-Type`; máy chủ không tin lời khai đó (D-086): nội dung
//! HTML khai là `image/png` mà lọt qua thì trình duyệt có thể chạy nó. Ở đây tự viết vài
//! chục dòng nhận diện cho các loại thư viện media nhận; kích thước ảnh (JPEG, PNG, GIF,
//! WebP, AVIF) đọc bằng thư viện `imagesize` — chỉ đọc header, không giải mã ảnh.

/// Số byte đầu cần để nhận diện loại.
pub(crate) const SNIFF_BYTES: usize = 1024;

/// Số byte đầu giữ lại để đọc kích thước ảnh (JPEG có thể có nhiều đoạn metadata trước
/// phần ghi kích thước).
pub(crate) const HEAD_BYTES: usize = 1024 * 1024;

/// Ảnh raster trình duyệt hiển thị được: chỉ những loại này được mở ngay trong trang
/// (`inline`) và có chiều rộng / cao.
pub(crate) const RASTER_IMAGES: [&str; 5] =
    ["image/jpeg", "image/png", "image/gif", "image/webp", "image/avif"];

pub(crate) fn is_raster_image(content_type: &str) -> bool {
    RASTER_IMAGES.contains(&content_type)
}

/// Loại nhận ra từ `head` (các byte đầu của nội dung), hoặc `None` nếu không nhận ra.
pub(crate) fn detect(head: &[u8]) -> Option<&'static str> {
    let starts = |signature: &[u8]| head.starts_with(signature);
    let at = |offset: usize, signature: &[u8]| {
        head.get(offset..offset + signature.len()) == Some(signature)
    };
    if starts(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if starts(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if starts(b"GIF87a") || starts(b"GIF89a") {
        return Some("image/gif");
    }
    if starts(b"RIFF") {
        if at(8, b"WEBP") {
            return Some("image/webp");
        }
        if at(8, b"WAVE") {
            return Some("audio/wav");
        }
        if at(8, b"AVI ") {
            return Some("video/x-msvideo");
        }
    }
    if at(4, b"ftyp") {
        return Some(iso_media(head));
    }
    if starts(&[0x1A, 0x45, 0xDF, 0xA3]) {
        // Matroska; WebM là Matroska với `DocType` = `webm` ngay trong header.
        let window = &head[..head.len().min(64)];
        let webm = window.windows(4).any(|part| part == b"webm");
        return Some(if webm { "video/webm" } else { "video/x-matroska" });
    }
    if starts(b"OggS") {
        return Some("audio/ogg");
    }
    if starts(b"fLaC") {
        return Some("audio/flac");
    }
    if starts(b"ID3") {
        return Some("audio/mpeg");
    }
    if let [0xFF, second, ..] = head {
        // Khung âm thanh MPEG: 11 bit đồng bộ. Lớp (2 bit) = 0 là AAC (ADTS), còn lại MP3.
        if second & 0xE0 == 0xE0 {
            return Some(if second & 0x06 == 0 { "audio/aac" } else { "audio/mpeg" });
        }
    }
    if starts(b"%PDF-") {
        return Some("application/pdf");
    }
    if starts(b"PK\x03\x04") {
        return Some("application/zip");
    }
    markup(head)
}

/// Loại của file thuộc họ ISO media (`ftyp`): ảnh AVIF / HEIC, âm thanh M4A, video MP4 / MOV.
fn iso_media(head: &[u8]) -> &'static str {
    let size = head
        .get(..4)
        .map_or(0, |bytes| u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize);
    let end = size.clamp(12, head.len());
    // Thương hiệu chính (byte 8–12) rồi các thương hiệu tương thích (từ byte 16).
    let major: [u8; 4] =
        head.get(8..12).and_then(|bytes| bytes.try_into().ok()).unwrap_or_default();
    let (compatible, _) = head.get(16..end).unwrap_or_default().as_chunks::<4>();
    let brands: Vec<[u8; 4]> = std::iter::once(major).chain(compatible.iter().copied()).collect();
    let any = |names: &[&[u8; 4]]| brands.iter().any(|brand| names.contains(&brand));
    if any(&[b"avif", b"avis"]) {
        "image/avif"
    } else if any(&[b"heic", b"heix", b"heim", b"heis", b"hevc", b"hevx", b"mif1", b"msf1"]) {
        "image/heic"
    } else if any(&[b"M4A ", b"M4B ", b"M4P "]) && !any(&[b"M4V "]) {
        "audio/mp4"
    } else if &major == b"qt  " {
        "video/quicktime"
    } else {
        "video/mp4"
    }
}

/// Văn bản đánh dấu: SVG, HTML, XML.
fn markup(head: &[u8]) -> Option<&'static str> {
    let text = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head);
    let text = String::from_utf8_lossy(&text[..text.len().min(SNIFF_BYTES)]).to_ascii_lowercase();
    let text = text.trim_start();
    if !text.starts_with('<') {
        return None;
    }
    const HTML: [&str; 8] =
        ["<!doctype html", "<html", "<head", "<body", "<script", "<iframe", "<meta", "<title"];
    if HTML.iter().any(|tag| text.contains(tag)) {
        return Some("text/html");
    }
    if text.contains("<svg") {
        return Some("image/svg+xml");
    }
    text.starts_with("<?xml").then_some("application/xml")
}

/// Tìm nội dung chạy được trong SVG — `<script>`, thuộc tính `on…=`, liên kết `javascript:`,
/// phần nhúng HTML — trên TOÀN BỘ file, từng mảnh khi đang nhận (D-112). Máy chủ đã trả SVG
/// dạng tải về kèm CSP chặn script, nhưng file có thể được phục vụ chỗ khác (CDN, website
/// đọc thẳng nơi lưu): không nhận thì không lo.
#[derive(Debug, Default)]
pub(crate) struct ActiveMarkup {
    /// Đuôi của mảnh trước, để mẫu nằm vắt qua hai mảnh vẫn thấy.
    tail: Vec<u8>,
    pub(crate) found: bool,
}

/// Độ dài phần đuôi giữ lại: dài hơn mẫu dài nhất.
const ACTIVE_TAIL: usize = 32;

impl ActiveMarkup {
    pub(crate) fn feed(&mut self, chunk: &[u8]) {
        if self.found {
            return;
        }
        let mut window = std::mem::take(&mut self.tail);
        window.extend(chunk.iter().map(u8::to_ascii_lowercase));
        self.found = has_active_markup(&window);
        let keep = window.len().saturating_sub(ACTIVE_TAIL);
        self.tail = window.split_off(keep);
    }
}

fn has_active_markup(text: &[u8]) -> bool {
    const TAGS: [&[u8]; 7] = [
        b"<script",
        b"javascript:",
        b"vbscript:",
        b"<foreignobject",
        b"<iframe",
        b"<embed",
        b"<object",
    ];
    if TAGS.iter().any(|tag| text.windows(tag.len()).any(|window| window == *tag)) {
        return true;
    }
    // Thuộc tính sự kiện: khoảng trắng hay `/`, rồi `on` + chữ cái, rồi `=` (có thể cách).
    text.windows(3).enumerate().any(|(index, window)| {
        let starts = window[0].is_ascii_whitespace() || window[0] == b'/';
        if !(starts && &window[1..] == b"on") {
            return false;
        }
        let rest = &text[index + 3..];
        let name = rest.iter().take_while(|byte| byte.is_ascii_alphabetic()).count();
        name > 0
            && rest[name..]
                .iter()
                .find(|byte| !byte.is_ascii_whitespace())
                .is_some_and(|b| *b == b'=')
    })
}

/// Các tên khác của cùng một loại; tên đầu nhóm là tên chuẩn, được lưu.
const SYNONYMS: [&[&str]; 9] = [
    &["image/jpeg", "image/jpg", "image/pjpeg"],
    &["audio/wav", "audio/x-wav", "audio/wave", "audio/vnd.wave"],
    &["audio/mpeg", "audio/mp3"],
    &["audio/mp4", "audio/x-m4a", "audio/m4a"],
    &["audio/flac", "audio/x-flac"],
    &["audio/aac", "audio/x-aac"],
    &["application/xml", "text/xml"],
    &["image/heic", "image/heif"],
    &["video/x-msvideo", "video/avi"],
];

/// Một định dạng chứa (container) mang được hình hay chỉ tiếng: chữ ký như nhau, khai loại
/// nào trong nhóm cũng khớp.
const CONTAINERS: [&[&str]; 4] = [
    &["video/mp4", "audio/mp4"],
    &["audio/ogg", "video/ogg", "application/ogg"],
    &["video/webm", "audio/webm"],
    &["video/x-matroska", "audio/x-matroska"],
];

/// Tên chuẩn của loại `content_type` (vd `image/jpg` → `image/jpeg`).
pub(crate) fn canonical(content_type: &str) -> &str {
    SYNONYMS
        .iter()
        .find(|group| group.contains(&content_type))
        .map_or(content_type, |group| group[0])
}

/// Khai `declared` có khớp loại nhận ra `detected` không.
pub(crate) fn compatible(declared: &str, detected: &str) -> bool {
    let declared = canonical(declared);
    if declared == detected {
        return true;
    }
    if CONTAINERS.iter().any(|group| group.contains(&declared) && group.contains(&detected)) {
        return true;
    }
    // Tài liệu Office / OpenDocument / EPUB là file zip.
    detected == "application/zip"
        && (declared.ends_with("+zip")
            || declared.starts_with("application/vnd.openxmlformats-officedocument.")
            || declared.starts_with("application/vnd.oasis.opendocument."))
}

/// Loại `declared` có chữ ký mà [`detect`] nhận ra được không. Loại có chữ ký thì nội
/// dung phải có đúng chữ ký đó; loại không có (vd `text/plain`) thì không kiểm được.
pub(crate) fn has_signature(declared: &str) -> bool {
    const DETECTABLE: [&str; 18] = [
        "image/jpeg",
        "image/png",
        "image/gif",
        "image/webp",
        "image/avif",
        "image/heic",
        "image/svg+xml",
        "audio/wav",
        "audio/ogg",
        "audio/flac",
        "audio/mpeg",
        "audio/aac",
        "audio/mp4",
        "video/webm",
        "video/x-matroska",
        "video/quicktime",
        "video/x-msvideo",
        "application/pdf",
    ];
    DETECTABLE.iter().any(|known| compatible(declared, known))
        || matches!(declared, "text/html" | "application/zip")
}

/// Chiều rộng và cao (pixel) của ảnh raster trong `head`; `None` nếu không đọc được.
pub(crate) fn dimensions(head: &[u8]) -> Option<(i32, i32)> {
    let size = imagesize::blob_size(head).ok()?;
    let width = i32::try_from(size.width).ok().filter(|width| *width > 0)?;
    let height = i32::try_from(size.height).ok().filter(|height| *height > 0)?;
    Some((width, height))
}

#[cfg(test)]
pub(crate) mod samples {
    //! Nội dung nhỏ nhất mà đúng định dạng, cho test: header thật, không có ảnh thật.

    pub fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
        bytes.extend(width.to_be_bytes());
        bytes.extend(height.to_be_bytes());
        bytes.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
        bytes.extend(b"\x00\x00\x00\x00IEND\xaeB`\x82");
        bytes
    }

    pub fn gif(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = b"GIF89a".to_vec();
        bytes.extend(width.to_le_bytes());
        bytes.extend(height.to_le_bytes());
        bytes.extend([0, 0, 0, b';']);
        bytes
    }

    pub fn jpeg(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        bytes.extend(b"JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00");
        bytes.extend([0xFF, 0xC0, 0x00, 0x11, 0x08]);
        bytes.extend(height.to_be_bytes());
        bytes.extend(width.to_be_bytes());
        bytes.extend([3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1, 0xFF, 0xD9]);
        bytes
    }

    pub fn webp(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"RIFF\x16\x00\x00\x00WEBPVP8X\x0a\x00\x00\x00\x00\x00\x00\x00".to_vec();
        bytes.extend(&(width - 1).to_le_bytes()[..3]);
        bytes.extend(&(height - 1).to_le_bytes()[..3]);
        bytes
    }

    pub fn avif(width: u32, height: u32) -> Vec<u8> {
        let boxed = |name: &[u8], body: Vec<u8>| {
            let mut bytes = u32::try_from(body.len() + 8).unwrap().to_be_bytes().to_vec();
            bytes.extend(name);
            bytes.extend(body);
            bytes
        };
        let mut ispe = vec![0, 0, 0, 0];
        ispe.extend(width.to_be_bytes());
        ispe.extend(height.to_be_bytes());
        let ipco = boxed(b"ipco", boxed(b"ispe", ispe));
        let mut meta = vec![0, 0, 0, 0];
        meta.extend(boxed(b"iprp", ipco));
        let mut bytes = boxed(b"ftyp", b"avif\x00\x00\x00\x00avifmif1".to_vec());
        bytes.extend(boxed(b"meta", meta));
        bytes
    }

    pub const SVG: &[u8] =
        b"<?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4\"/>";
    pub const HTML: &[u8] = b"<!DOCTYPE html><html><script>alert(1)</script></html>";
    pub const PDF: &[u8] = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n1 0 obj\n<<>>\nendobj\n%%EOF\n";
    pub const MP4: &[u8] = b"\x00\x00\x00\x18ftypisom\x00\x00\x02\x00isomiso2\x00\x00\x00\x08free";
}

#[cfg(test)]
mod tests {
    use super::samples::*;
    use super::*;

    #[test]
    fn tim_noi_dung_chay_duoc_trong_svg_ca_khi_vat_qua_hai_manh() {
        let scan = |parts: &[&str]| {
            let mut scanner = ActiveMarkup::default();
            for part in parts {
                scanner.feed(part.as_bytes());
            }
            scanner.found
        };
        assert!(!scan(&["<svg viewBox=\"0 0 1 1\"><path d=\"M0 0\" stroke=\"red\"/></svg>"]));
        assert!(!scan(&["<svg><text>Don't wait, online now</text></svg>"]));
        assert!(scan(&["<svg>", "<SCRIPT>alert(1)</script>"]));
        assert!(scan(&["<svg><scr", "ipt>"]));
        assert!(scan(&["<svg onload", " = \"x()\">"]));
        assert!(scan(&["<svg><a href=\"JavaScript:x()\">"]));
        assert!(scan(&["<svg><foreignObject><div/></foreignObject>"]));
    }

    #[test]
    fn nhan_dien_loai_that() {
        let cases: [(Vec<u8>, &str); 14] = [
            (png(1, 1), "image/png"),
            (jpeg(1, 1), "image/jpeg"),
            (gif(1, 1), "image/gif"),
            (webp(1, 1), "image/webp"),
            (avif(1, 1), "image/avif"),
            (SVG.to_vec(), "image/svg+xml"),
            (HTML.to_vec(), "text/html"),
            (PDF.to_vec(), "application/pdf"),
            (MP4.to_vec(), "video/mp4"),
            (b"\x1aE\xdf\xa3\x9fB\x86\x81\x01B\x82\x84webm".to_vec(), "video/webm"),
            (b"OggS\x00\x02".to_vec(), "audio/ogg"),
            (b"ID3\x04\x00".to_vec(), "audio/mpeg"),
            (b"RIFF\x24\x00\x00\x00WAVEfmt ".to_vec(), "audio/wav"),
            (b"\xef\xbb\xbf  <svg viewBox=\"0 0 1 1\"></svg>".to_vec(), "image/svg+xml"),
        ];
        for (content, expected) in cases {
            assert_eq!(detect(&content), Some(expected), "{expected}");
        }
        assert_eq!(detect(b"hello world"), None);
        assert_eq!(detect(b""), None);
        // SVG có nhúng HTML: coi là HTML.
        assert_eq!(
            detect(b"<svg><foreignObject><body></body></foreignObject></svg>"),
            Some("text/html")
        );
    }

    #[test]
    fn loai_khai_khop_loai_that() {
        assert!(compatible("image/jpg", "image/jpeg"));
        assert_eq!(canonical("image/pjpeg"), "image/jpeg");
        assert_eq!(canonical("video/mp4"), "video/mp4");
        assert!(compatible("audio/x-m4a", "video/mp4"));
        assert!(compatible("audio/mp4", "video/mp4"));
        assert!(compatible("application/epub+zip", "application/zip"));
        assert!(!compatible("image/png", "text/html"));
        assert!(!compatible("image/png", "image/jpeg"));
        assert!(has_signature("image/png"));
        assert!(has_signature("audio/x-wav"));
        assert!(!has_signature("text/plain"));
    }

    #[test]
    fn kich_thuoc_tung_dinh_dang_anh() {
        assert_eq!(dimensions(&png(640, 480)), Some((640, 480)));
        assert_eq!(dimensions(&jpeg(1920, 1080)), Some((1920, 1080)));
        assert_eq!(dimensions(&gif(32, 16)), Some((32, 16)));
        assert_eq!(dimensions(&webp(300, 200)), Some((300, 200)));
        assert_eq!(dimensions(&avif(4000, 3000)), Some((4000, 3000)));
        assert_eq!(dimensions(b"\x89PNG\r\n\x1a\n"), None);
    }
}
