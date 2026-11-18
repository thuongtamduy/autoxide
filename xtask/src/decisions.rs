//! Mục lục nhật ký quyết định: `cargo xtask decisions` sinh `docs/decisions/INDEX.md` từ
//! `docs/DECISIONS.md` (mục cũ D-001…, đóng) và `docs/decisions/D-<ngày>-<tên>.md` (mỗi mục mới một
//! file, D-20261118-decision-ids); `cargo xtask check` báo khi file đã commit lệch. Luật về ID (không
//! trùng, đúng dạng, tham chiếu có thật) ở `decisions/rules.rs`.
//!
//! Nhật ký dài (hàng trăm mục, hàng nghìn dòng): agent đọc mục lục để biết đã quyết gì về một chủ
//! đề, mục nào đã bị thay, rồi mới mở đúng mục. Mục lục sinh từ chính nhật ký nên không lệch được:
//!
//! - **chủ đề** — dòng `Chủ đề: <tên>` của mục nếu có (phải là một tên trong [`TOPICS`]); không có
//!   thì đoán từ từ khoá trong tiêu đề, theo thứ tự của [`TOPICS`] (chủ đề hẹp trước, rộng sau);
//! - **đã bị thay** — mục sau có đoạn bắt đầu bằng `Thay` (`Thay: D-xxx`, `Thay D-xxx (…)`, `Thay:
//!   phần … của D-xxx`), hoặc viết `(thay D-xxx)` / `thay phần … của D-xxx` giữa câu. Chỉ tính số
//!   nằm trước dấu `—` hay hết câu đầu, và không tính số trong ngoặc (thường là lý do, không phải
//!   mục bị thay). Có chữ khác ngoài danh sách số (`phần`, `câu`, `mục`…) thì là thay một phần.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use crate::boundaries::Violation;

mod rules;

/// Nhật ký cũ (D-001…, mỗi mục một số tuần tự), tính từ gốc repo. Đóng: mục mới là file riêng.
pub const LOG_FILE: &str = "docs/DECISIONS.md";
/// Thư mục của mục mới (`D-<YYYYMMDD>-<tên>.md`, một mục một file) và của mục lục.
pub const DIR: &str = "docs/decisions";
/// Mục lục sinh ra, tính từ gốc repo.
pub const INDEX_FILE: &str = "docs/decisions/INDEX.md";

/// Chủ đề, theo thứ tự trong mục lục, và từ khoá để đoán (chữ thường, ngăn bằng `|`, khớp trọn từ
/// trong tiêu đề). Chủ đề cuối là chỗ cho mục không khớp gì.
pub const TOPICS: &[(&str, &str)] = &[
    (
        SCOPE,
        "phạm vi core|mục tiêu|mục đích sản phẩm|công khai repo|công khai toàn bộ|riêng tư|tiêu chí|xuyên suốt|\
        core chỉ có api|mã nguồn mở|nền cho kinh doanh|sản phẩm có cửa|tên chỗ đặt",
    ),
    (
        PROCESS,
        "gate|lộ trình|phát hành|merge|ci|github|dependabot|rustsec|toolchain|edition|subagent|\
        xtask|gitleaks|đặc tả|license|bảo trì|semver|sdk|test|lint|ngôn ngữ tài liệu|tài liệu tiếng việt|\
        hướng dẫn|để sau|chuyển sang bước|bản release|build|nhịp|tự phát hành|rà bước|gom bước|rà soát độc lập",
    ),
    (ARCHITECTURE, "module|kernel|trait|ranh giới|workspace|stack|binary|hợp đồng|lệnh|cli|task"),
    (
        DATABASE,
        "database|sqlite|postgres|migration|seaorm|sql|uuidv7|id|index|khoá dòng|job|event|outbox|\
        audit|backup|đánh số|tìm|tìm kiếm|lọc|sse|realtime|dữ liệu|chép|xung đột|single writer",
    ),
    (
        HTTP,
        "http|rest|openapi|endpoint|api|response|idempotency|etag|deprecated|header|lỗi|i18n|dịch|\
        pagination|health|cors|tải|tcp_nodelay|chống trùng",
    ),
    (AI, "mcp|ai|llms|prompt|prompts|embedder|lời thường"),
    (
        AUTH,
        "đăng nhập|mật khẩu|jwt|totp|mfa|passkey|recovery|role|permission|phân quyền|quyền|api key|\
        token|oidc|bảo mật|secret|khoá ký|mã hoá|throttle|throttling|rate|spam|altcha|ip|proxy|tài khoản|\
        danh tính|người dùng|user|membership|xác minh|email|credential",
    ),
    (
        TENANCY,
        "khách|đa khách|tenant|placement|tổ chức|nút|website|thuê bao|hạn mức|quyền dùng|uỷ quyền|\
        multi",
    ),
    (
        INTERFACE,
        "đề bài giao diện|giao diện|thiết kế 2|thiết kế giao diện|đối chiếu|yêu cầu website|\
        hộp việc|lỗ nhỏ|điều hướng|trường tự định nghĩa",
    ),
    (
        CONTENT,
        "nội dung|trang|ảnh|thumbnail|webp|thông báo|hành chính|markdown|menu|file|mail|kênh đẩy|di động",
    ),
    (
        MONEY,
        "tiền|thuế|hoá đơn|chứng từ|sổ cái|thanh toán|tỉ giá|dòng tiền|cod|kế toán|rust_decimal|két",
    ),
    (
        LAW,
        "pháp luật|pháp lý|tuân thủ|bộ công thương|dữ liệu cá nhân|sở hữu trí tuệ|người tiêu dùng|\
        đồng ý|cookie|định danh|lưu trữ 10 năm|xác thực tài khoản đăng nội dung",
    ),
    (
        COMMERCE,
        "thương mại|sản phẩm|đơn|giỏ|kho|khuyến mại|sàn|bảng giá|người bán|cửa hàng|banner|\
        điểm thưởng|đánh giá|quầy|catalog|orders|shipping",
    ),
    (
        OPERATIONS,
        "vận hành|hiệu năng|đo|số đo|log|metrics|tracing|docker|deploy|cấu hình|doctor|tắt|s3|\
        khoảng rút",
    ),
    (OTHER, ""),
];

const SCOPE: &str = "Phạm vi và mục tiêu";
const PROCESS: &str = "Quy trình, gate, lộ trình";
const ARCHITECTURE: &str = "Kiến trúc module và kernel";
const DATABASE: &str = "Database, dữ liệu, job, event";
const HTTP: &str = "HTTP, REST, OpenAPI";
const AI: &str = "MCP, AI, agent dùng API";
const AUTH: &str = "Xác thực, phân quyền, bảo mật";
const TENANCY: &str = "Đa khách, tổ chức, gói";
const INTERFACE: &str = "Hợp đồng cho giao diện";
const CONTENT: &str = "Module nội dung, file, liên lạc";
const MONEY: &str = "Tiền, thuế, chứng từ";
const LAW: &str = "Pháp luật và tuân thủ";
const COMMERCE: &str = "Thương mại";
const OPERATIONS: &str = "Vận hành và hiệu năng";
const OTHER: &str = "Khác";

/// Thứ tự đoán: chủ đề hẹp trước. `None` là bước tra tên module trong tiêu đề ([`MODULES`]): mục
/// nói về một module thì về chủ đề của module đó, trừ khi đã khớp một chủ đề đứng trước.
const GUESS_ORDER: &[Option<&str>] = &[
    Some(SCOPE),
    Some(PROCESS),
    Some(INTERFACE),
    None,
    Some(LAW),
    Some(MONEY),
    Some(AI),
    Some(COMMERCE),
    Some(TENANCY),
    Some(AUTH),
    Some(CONTENT),
    Some(HTTP),
    Some(DATABASE),
    Some(OPERATIONS),
    Some(ARCHITECTURE),
];

/// Tên module trong tiêu đề → chủ đề.
const MODULES: &[(&str, &str)] = &[
    ("auth", AUTH),
    ("content", CONTENT),
    ("cms", CONTENT),
    ("media", CONTENT),
    ("files", CONTENT),
    ("forms", CONTENT),
    ("newsletter", CONTENT),
    ("comments", CONTENT),
    ("menus", CONTENT),
    ("address", CONTENT),
    ("notifications", CONTENT),
    ("webhooks", CONTENT),
    ("mail", CONTENT),
    ("ledger", MONEY),
    ("invoices", MONEY),
    ("tax", MONEY),
    ("compliance", LAW),
    ("plans", TENANCY),
    ("organizations", TENANCY),
    ("ai", AI),
];

/// Một mục của nhật ký.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub date: String,
    /// `chủ dự án quyết` hay `agent quyết` (nguyên văn trong tiêu đề).
    pub who: String,
    /// Dòng `Chủ đề:` của mục, nếu có.
    pub topic: Option<String>,
    /// Mục mà mục này thay: (ID, chỉ thay một phần).
    pub replaces: Vec<(String, bool)>,
    /// File chứa mục, tính từ gốc repo.
    pub file: String,
    /// Dòng của tiêu đề trong file (đếm từ 1), cho thông báo lỗi.
    pub line: usize,
}

/// Đọc các mục của một file nhật ký. Bỏ qua tiêu đề nằm trong khối code (mẫu ở đầu file).
pub fn parse(text: &str) -> Result<Vec<Entry>, String> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut body: Vec<&str> = Vec::new();
    let mut fenced = false;
    for (index, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        if !fenced && line.starts_with("## D-") {
            if let Some(entry) = entries.last_mut() {
                finish(entry, &body);
            }
            body.clear();
            entries.push(heading(line, index + 1)?);
        } else if !entries.is_empty() {
            body.push(line);
        }
    }
    if let Some(entry) = entries.last_mut() {
        finish(entry, &body);
    }
    Ok(entries)
}

/// `## <ID> · <tiêu đề> — <YYYY-MM-DD> — <ai quyết>`.
fn heading(line: &str, number: usize) -> Result<Entry, String> {
    let bad = || {
        format!(
            "line {number}: `{line}` — a decision heading is `## <ID> · <title> — <YYYY-MM-DD> — \
             <chủ dự án quyết | agent quyết>`"
        )
    };
    let rest = line.strip_prefix("## ").ok_or_else(bad)?;
    let (id, rest) = rest.split_once(" · ").ok_or_else(bad)?;
    // Tiêu đề có thể chứa ` — `: tách hai phần cuối từ bên phải.
    let mut parts = rest.rsplitn(3, " — ");
    let who = parts.next().ok_or_else(bad)?;
    let date = parts.next().ok_or_else(bad)?;
    let title = parts.next().ok_or_else(bad)?;
    if !is_date(date) {
        return Err(bad());
    }
    Ok(Entry {
        id: id.trim().to_owned(),
        title: title.trim().to_owned(),
        date: date.to_owned(),
        who: who.trim().to_owned(),
        topic: None,
        replaces: Vec::new(),
        file: String::new(),
        line: number,
    })
}

fn is_date(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            _ => byte.is_ascii_digit(),
        })
}

/// Điền chủ đề và các mục bị thay từ thân của mục.
fn finish(entry: &mut Entry, body: &[&str]) {
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in body {
        if line.trim().is_empty() {
            if !current.is_empty() {
                paragraphs.push(std::mem::take(&mut current));
            }
        } else {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(line.trim());
        }
    }
    if !current.is_empty() {
        paragraphs.push(current);
    }
    for paragraph in &paragraphs {
        if let Some(topic) = paragraph.strip_prefix("Chủ đề:") {
            entry.topic = Some(topic.trim().trim_end_matches('.').to_owned());
        }
        if let Some(clause) = replace_clause(paragraph) {
            add_replaced(entry, &clause);
        }
        for clause in inline_replacements(paragraph) {
            add_replaced(entry, &clause);
        }
    }
}

fn add_replaced(entry: &mut Entry, clause: &str) {
    let ids = ids_in(clause);
    let partial = clause_is_partial(clause, &ids);
    for id in ids {
        if id != entry.id && !entry.replaces.iter().any(|(known, _)| *known == id) {
            entry.replaces.push((id, partial));
        }
    }
}

/// Đoạn bắt đầu bằng `Thay` (`Thay:`, `Thay D-…`, `Thay một phần D-…`), cắt tới `—` hay hết câu
/// đầu, bỏ phần trong ngoặc. `Thay thế: không.` thì không có gì.
fn replace_clause(paragraph: &str) -> Option<String> {
    let rest = paragraph.strip_prefix("Thay")?;
    if !(rest.starts_with(':') || rest.starts_with(' ')) {
        return None;
    }
    if rest.trim_start().starts_with("thế: không") {
        return None;
    }
    Some(first_clause(rest))
}

/// `(thay D-xxx)` và `thay phần … của D-xxx` giữa câu.
fn inline_replacements(paragraph: &str) -> Vec<String> {
    let mut clauses = Vec::new();
    let mut search = paragraph;
    while let Some(position) = search.find("thay ") {
        let before = search[..position].chars().next_back();
        let after = &search[position + "thay ".len()..];
        // Trọn từ `thay` (không phải `thay đổi`, `thay được`): theo sau là `D-`, `cho D-` hay `phần`.
        let starts_word = before.is_none_or(|c| !c.is_alphanumeric());
        if starts_word
            && (after.starts_with("D-")
                || after.starts_with("cho D-")
                || after.starts_with("phần "))
        {
            let clause = first_clause(after);
            // `thay phần …` phải nêu mục bị thay ngay trong câu đó.
            if !ids_in(&clause).is_empty() {
                clauses.push(format!(
                    "{}{clause}",
                    if after.starts_with("phần ") { "phần " } else { "" }
                ));
            }
        }
        search = after;
    }
    clauses
}

/// Phần đầu của câu: tới `—`, `:`, `. ` hay `)` đóng ngoặc ngoài; bỏ phần trong ngoặc.
fn first_clause(text: &str) -> String {
    let mut clause = String::new();
    let mut depth = 0usize;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '(' => depth += 1,
            ')' if depth == 0 => break,
            ')' => depth -= 1,
            '—' if depth == 0 => break,
            '.' if depth == 0 && chars.peek().is_none_or(|next| next.is_whitespace()) => break,
            // `Thay:` mở đầu thì bỏ; `Thay D-087 (…): lý do` thì hết phần nêu mục bị thay.
            ':' if depth == 0 && clause.trim().is_empty() => {}
            ':' if depth == 0 => break,
            _ if depth == 0 => clause.push(c),
            _ => {}
        }
    }
    clause.trim().to_owned()
}

/// Các ID trong một đoạn: `D-` + ba chữ số, hay ID theo ngày `D-YYYYMMDD-<tên>`.
pub fn ids_in(text: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let bytes = text.as_bytes();
    let mut index = 0;
    while let Some(offset) = text[index..].find("D-") {
        let start = index + offset;
        index = start + 2;
        if start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
            continue;
        }
        if let Some(id) = id_at(&text[start..]) {
            index = start + id.len();
            ids.push(id);
        }
    }
    ids
}

/// ID ở đầu `text` (bắt đầu bằng `D-`), nếu có.
fn id_at(text: &str) -> Option<String> {
    let rest = text.strip_prefix("D-")?;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    let next = rest.as_bytes().get(digits).copied();
    match digits {
        3 if next.is_none_or(|byte| !byte.is_ascii_alphanumeric()) => {
            Some(format!("D-{}", &rest[..3]))
        }
        8 if next == Some(b'-') => {
            let slug: String = rest[9..]
                .chars()
                .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
                .collect();
            let slug = slug.trim_end_matches('-');
            (!slug.is_empty()).then(|| format!("D-{}-{slug}", &rest[..8]))
        }
        _ => None,
    }
}

/// Câu chỉ có danh sách ID (và `và`, `cho`) là thay trọn; có chữ khác là thay một phần.
fn clause_is_partial(clause: &str, ids: &[String]) -> bool {
    let mut rest = clause.to_owned();
    for id in ids {
        rest = rest.replace(id.as_str(), " ");
    }
    rest.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .any(|word| !matches!(word, "và" | "cho" | "D"))
}

/// Chủ đề của mục: dòng `Chủ đề:` nếu có, không thì đoán từ tiêu đề.
pub fn topic_of(entry: &Entry) -> &'static str {
    if let Some(topic) = &entry.topic
        && let Some((name, _)) = TOPICS.iter().find(|(name, _)| name == topic)
    {
        return name;
    }
    guess_topic(&entry.title)
}

fn guess_topic(title: &str) -> &'static str {
    // Đệm khoảng trắng hai đầu mỗi từ để khớp trọn từ: ` ai ` không khớp ` mail `.
    let words: Vec<String> = title
        .to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect();
    let normalized = format!(" {} ", words.join(" "));
    let has = |keyword: &str| normalized.contains(&format!(" {keyword} "));
    for step in GUESS_ORDER {
        match step {
            Some(topic) => {
                let keywords = TOPICS.iter().find(|(name, _)| name == topic).map(|(_, k)| *k);
                if keywords.unwrap_or_default().split('|').map(str::trim).any(&has) {
                    return topic;
                }
            }
            // Module nhắc tới đầu tiên trong tiêu đề.
            None => {
                let first =
                    words.iter().find_map(|word| MODULES.iter().find(|(module, _)| module == word));
                if let Some((_, topic)) = first {
                    return topic;
                }
            }
        }
    }
    OTHER
}

/// Mọi mục: nhật ký cũ theo thứ tự trong file, rồi mục mới theo ID (ngày, rồi tên).
pub fn load(root: &Path) -> Result<Vec<Entry>, String> {
    let mut entries = read_file(root, LOG_FILE)?;
    for file in rules::entry_files(root)? {
        entries.extend(read_file(root, &file)?);
    }
    Ok(entries)
}

fn read_file(root: &Path, file: &str) -> Result<Vec<Entry>, String> {
    let text = fs::read_to_string(root.join(file)).map_err(|error| format!("{file}: {error}"))?;
    let mut entries = parse(&text).map_err(|error| format!("{file}: {error}"))?;
    for entry in &mut entries {
        entry.file = file.to_owned();
    }
    Ok(entries)
}

/// Nội dung mục lục.
pub fn render(entries: &[Entry]) -> String {
    // Mục bị thay → các mục thay nó.
    let mut replaced_by: BTreeMap<&str, Vec<(&str, bool)>> = BTreeMap::new();
    for entry in entries {
        for (id, partial) in &entry.replaces {
            replaced_by.entry(id.as_str()).or_default().push((entry.id.as_str(), *partial));
        }
    }
    let mut text = String::new();
    let _ = writeln!(text, "# Mục lục quyết định\n");
    let _ = writeln!(
        text,
        "> Sinh bằng `cargo xtask decisions` từ `{LOG_FILE}` và `{DIR}/D-*.md` — **không sửa \
         tay**; `cargo xtask check` (trong gate) báo khi file này lệch.\n"
    );
    let _ = writeln!(
        text,
        "Ghi quyết định mới: tạo file `{DIR}/D-<YYYYMMDD>-<tên-ngắn>.md` (một mục một file, ID theo \
         ngày và tên nên hai phiên song song không chọn trùng), rồi chạy `cargo xtask decisions`. \
         Mẫu và luật: skill `record-decision`. `{LOG_FILE}` là nhật ký cũ (D-001…), không thêm mục \
         vào đó.\n"
    );
    let _ = writeln!(
        text,
        // Không ghi số mục (tổng hay theo chủ đề): hai nhánh song song cùng thêm mục thì git gộp
        // sạch mà số đếm sai, gate báo lệch dù không ai sửa tay. Không có số đếm thì hai mục thêm ở
        // hai chỗ gộp vẫn đúng; thêm sát nhau thì git báo xung đột, sinh lại là xong.
        "Gom theo chủ đề (dòng `Chủ đề:` của mục, không có thì đoán từ tiêu đề). Mỗi dòng: \
         số, tiêu đề, ngày, ai quyết — mục **chủ dự án quyết** agent không tự đảo. \"Thay bởi\" \
         nghĩa là mục sau đã đổi mục này: đọc mục sau trước. Nội dung đầy đủ của mục cũ: `grep -n \
         '^## D-123 ' {LOG_FILE}`; mục mới có link tới file của nó.\n"
    );
    for (name, _) in TOPICS {
        let members: Vec<&Entry> =
            entries.iter().filter(|entry| topic_of(entry) == *name).collect();
        if members.is_empty() {
            continue;
        }
        let _ = writeln!(text, "## {name}\n");
        for entry in members {
            let who = if entry.who.starts_with("chủ dự án") {
                " · **chủ dự án quyết**"
            } else {
                ""
            };
            if entry.file == LOG_FILE {
                let _ = write!(text, "- **{}** {} — {}{who}", entry.id, entry.title, entry.date);
            } else {
                // Link tương đối: mục lục nằm cùng thư mục với file của mục.
                let name = entry.file.rsplit('/').next().unwrap_or_default();
                let (id, title, date) = (&entry.id, &entry.title, &entry.date);
                let _ = write!(text, "- **[{id}]({name})** {title} — {date}{who}");
            }
            if let Some(by) = replaced_by.get(entry.id.as_str()) {
                let full: Vec<String> = by
                    .iter()
                    .filter(|(_, partial)| !partial)
                    .map(|(id, _)| (*id).to_owned())
                    .collect();
                let partial: Vec<String> = by
                    .iter()
                    .filter(|(_, partial)| *partial)
                    .map(|(id, _)| (*id).to_owned())
                    .collect();
                if !full.is_empty() {
                    let _ = write!(text, " · *thay bởi {}*", full.join(", "));
                }
                if !partial.is_empty() {
                    let _ = write!(text, " · *thay một phần bởi {}*", partial.join(", "));
                }
            }
            let _ = writeln!(text);
        }
        let _ = writeln!(text);
    }
    // Một dòng trống cuối file, không hai.
    while text.ends_with("\n\n") {
        text.pop();
    }
    text
}

/// Ghi mục lục (`cargo xtask decisions`). Trả số mục.
pub fn write_index(root: &Path) -> Result<usize, String> {
    let entries = load(root)?;
    let path = root.join(INDEX_FILE);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("{INDEX_FILE}: {error}"))?;
    }
    fs::write(&path, render(&entries)).map_err(|error| format!("{INDEX_FILE}: {error}"))?;
    Ok(entries.len())
}

/// Luật `decisions-index` (mục lục đã commit khớp nhật ký) và các luật về ID của `rules`.
pub fn check(root: &Path) -> Result<Vec<Violation>, String> {
    let entries = load(root)?;
    let mut violations = rules::check(root, &entries)?;
    let expected = render(&entries);
    let actual = fs::read_to_string(root.join(INDEX_FILE)).unwrap_or_default();
    if actual != expected {
        violations.push(Violation {
            rule: "decisions-index",
            location: INDEX_FILE.to_owned(),
            message: format!(
                "out of date with {LOG_FILE} and {DIR}/D-*.md — regenerate it with `cargo xtask \
                 decisions` and commit the result"
            ),
        });
    }
    Ok(violations)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "# Nhật ký\n\n```\n## D-NNN · <tiêu đề> — <ngày> — <ai>\n```\n\n\
## D-001 · Database: SQLite và Postgres — 2026-11-11 — chủ dự án quyết\n\nQuyết định: hai database.\n\n\
## D-002 · Tên — dự án — 2026-11-12 — agent quyết\n\nThay: phần \"tên\" của D-001 — vì (D-003).\n\n\
## D-003 · Gate — 2026-11-13 — agent quyết\n\nQuyết định (thay D-002): x.\n\nThay thế: không. Bổ sung D-001.\n\n\
## D-004 · Mục mới — 2026-11-14 — agent quyết\n\nChủ đề: Phạm vi và mục tiêu\n\nThay: D-003. Việc làm: bước 30 (D-001).\n";

    #[test]
    fn doc_tieu_de_chu_de_va_muc_bi_thay() {
        let entries = parse(LOG).unwrap();
        assert_eq!(entries.len(), 4, "khối code không tính");
        assert_eq!(entries[1].title, "Tên — dự án");
        assert_eq!(entries[1].replaces, vec![("D-001".to_owned(), true)]);
        assert_eq!(
            entries[2].replaces,
            vec![("D-002".to_owned(), false)],
            "`Thay thế: không` bỏ qua"
        );
        assert_eq!(entries[3].replaces, vec![("D-003".to_owned(), false)]);
        assert_eq!(topic_of(&entries[3]), "Phạm vi và mục tiêu");
        assert_eq!(topic_of(&entries[0]), DATABASE);
    }

    #[test]
    fn muc_luc_ghi_thay_boi() {
        let mut entries = parse(LOG).unwrap();
        entries.iter_mut().for_each(|entry| entry.file = LOG_FILE.to_owned());
        let mut new = entries[3].clone();
        new.id = "D-20261118-new-rule".to_owned();
        new.file = format!("{DIR}/D-20261118-new-rule.md");
        new.replaces.clear();
        entries.push(new);
        let index = render(&entries);
        assert!(
            index.contains("- **[D-20261118-new-rule](D-20261118-new-rule.md)** Mục mới"),
            "{index}"
        );
        assert!(index.contains("**D-001** Database: SQLite và Postgres — 2026-11-11 · **chủ dự án quyết** · *thay một phần bởi D-002*"), "{index}");
        assert!(index.contains("**D-003** Gate — 2026-11-13 · *thay bởi D-004*"), "{index}");
        assert!(index.ends_with("\n") && !index.ends_with("\n\n"));
    }

    #[test]
    fn nhan_id_dung_dang() {
        assert_eq!(
            ids_in("D-001, (D-12) D-0001 xD-002 D-20261118-decision-ids."),
            vec!["D-001".to_owned(), "D-20261118-decision-ids".to_owned()]
        );
    }

    #[test]
    fn tieu_de_sai_mau_bao_loi() {
        assert!(parse("## D-001 · Không có ngày — agent quyết\n").is_err());
    }
}
