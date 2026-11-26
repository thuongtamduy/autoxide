## D-20261126-content-word-index · Tìm chữ công khai của `content`: chỉ mục từ thật trên cả hai database, sau trait `WordIndex`, có trần thời gian câu lệnh — 2026-11-26 — agent quyết

Chủ đề: Vận hành và hiệu năng

Quyết định: tìm chữ trên bản công khai của `content` (danh sách bài `GET /public/posts?q=` và phần "theo chữ" của
`GET /public/search`) không còn dùng `search_key LIKE '%q%'` (quét mọi bài còn lại sau bộ lọc, PT-15). Thay bằng chỉ
mục từ, giấu sau trait riêng của module `WordIndex` (`modules/content/src/publishing/words.rs`), mỗi database một cài
đặt, chọn theo backend của kết nối:

- **Từ** do Rust tách, không do database: `words(text)` = `search::fold` (bỏ dấu, chữ thường) rồi cắt ở mọi ký tự
  không phải chữ / số (`char::is_alphanumeric`), bỏ từ dài quá 64 ký tự, nối bằng một dấu cách. Lưu ở cột mới
  `content_published.search_words` (ghi cùng lúc với `search_key` khi đăng; migration tính lại cho dòng cũ). Hai
  database chỉ thấy các từ chữ-số cách nhau bằng dấu cách, nên bộ tách từ của mỗi bên cho cùng một tập từ.
- **Câu tìm**: `WordQuery::parse(q)` tách bằng cùng hàm, bỏ từ trùng, tối đa 8 từ; mọi từ phải có (AND); từ từ 3 ký
  tự trở lên khớp **tiền tố** (`dala` tìm thấy `dalat`), từ 1–2 ký tự khớp nguyên từ (tiền tố ngắn khớp quá nhiều
  từ). Câu không còn từ nào thì không có kết quả.
- **Postgres**: index GIN trên biểu thức `to_tsvector('simple', search_words)`; điều kiện `@@ to_tsquery('simple',
  'dala:* & lat')` (từ chỉ có chữ-số nên không cần thoát ký tự). Cấu hình `simple`: không bỏ stop word, không cắt
  gốc từ — giống FTS5.
- **SQLite**: bảng ảo FTS5 `content_published_fts(search_words)` (`unicode61`, không bỏ dấu vì đã bỏ ở Rust), giữ
  đồng bộ bằng trigger trên `content_published`. `rowid` của FTS là cột mới `content_published.search_rowid`
  (trigger gán `MAX + 1` khi thêm dòng, có index), không dùng `rowid` của `content_published`: bảng có khoá chính
  ghép nên `VACUUM` được phép đánh số lại `rowid`, chỉ mục sẽ trỏ sai. Cột `search_rowid` có trên cả hai database
  (Postgres để `NULL`) để schema hai bên giống nhau khi chép dữ liệu (`autoxide copy`).
- **Hai bước**: (1) `WordIndex::matching` lấy tối đa `MATCH_CAP` (1 000) ID khớp, mới đăng trước, trong khách,
  ngôn ngữ, bỏ bài bị ẩn — câu duy nhất chạm chỉ mục, chạy dưới trần thời gian; (2) câu SeaORM thường như cũ
  (website, phân loại, tác giả, keyset pagination) thêm `entry_id IN (…)`. Kết quả tìm chữ vì thế chỉ phủ 1 000 bài
  khớp mới nhất — đủ cho ô tìm của website, và chặn chi phí của từ rất phổ biến.
- **Trần thời gian câu lệnh trên cả hai database** (điểm còn lại của #548): Postgres chạy bước (1) trong transaction
  đọc với `SET LOCAL statement_timeout`; SQLite chạy trên một kết nối lấy riêng từ pool đọc, đặt progress handler
  của SQLite trả "dừng" khi quá hạn (lỗi `SQLITE_INTERRUPT`), gỡ handler trước khi trả kết nối về pool. Quá hạn
  → `503 content.public_search_timeout` (mã đã có). Câu bước (1) viết bằng sqlx trực tiếp, không qua SeaORM: toán
  tử tìm chữ (`@@`, `MATCH`) không có trong sea-query, và trần thời gian cần chính kết nối chạy câu — SeaORM không
  cho lấy kết nối sqlx bên dưới. Câu chỉ nhận giá trị kiểu chuỗi, UUID, số nguyên.
- Kernel: `transfer::module_tables` trên SQLite bỏ bảng ảo và bảng phụ của chúng (`pragma_table_list` loại
  `virtual` / `shadow`): chúng là dữ liệu dẫn xuất, trigger dựng lại khi chép `content_published`. Chép SQLite →
  Postgres hay ngược lại không đổi gì khác.
- Tìm bản nháp (người biên tập, cần `content.read`) giữ `LIKE` như mọi ô tìm trong module (bảng nháp nhỏ hơn và
  không công khai).

Ba điểm còn lại của #548, cùng PR:

- `loaded()` (nạp vector của một khách / phạm vi / ngôn ngữ vào bộ nhớ) chạy **một lần** cho mỗi khoá dù nhiều
  lượt tìm cùng lúc thấy cache cũ (single-flight: một `tokio::sync::Mutex` theo khoá, lượt sau chờ rồi đọc cache).
- Lượt tìm công khai mà phần tìm theo ý nghĩa bị cắt vì quá giờ được **trả lại** lượt đã trừ ở ngân sách mỗi phút
  của khách (khách không mất ngân sách cho kết quả không nhận được).
- Phần duyệt vector chạy trên `spawn_blocking` nhận một cờ huỷ; future bị bỏ (quá giờ, client ngắt) thì cờ bật và
  vòng duyệt dừng sau tối đa 1 024 đoạn — việc CPU không còn chạy tiếp ngoài trần số lượt cùng lúc.

Lý do: PT-15 (đợt test 2026-11) — tìm công khai trên bảng rất lớn chậm, giới hạn theo client chưa chặn hết chi phí.
Chỉ mục từ đưa chi phí về theo số bài khớp, không theo số bài của khách; trần thời gian là chốt cuối khi từ khớp
quá nhiều.

Đã cân nhắc:

- Tách từ bằng database (`to_tsvector` trên `search_key`, `unicode61` trên `search_key`): bộ tách của Postgres nhận
  dạng email, URL, số thực, từ có gạch nối… khác FTS5, hai database cho kết quả khác nhau.
- FTS5 `content=` trỏ vào `content_published` theo `rowid`: hỏng sau `VACUUM` (xem trên). Bảng ánh xạ khoá riêng
  cho FTS: là bảng thường nên `autoxide copy` chép nó, đụng dòng trigger vừa tạo.
- Ghi chỉ mục từ Rust thay cho trigger: chép dữ liệu (`autoxide copy`, chuyển khách) sẽ không dựng lại chỉ mục.
- Trần thời gian bằng cách bỏ future: câu vẫn chạy tiếp trong database. Progress handler gắn khi mở mọi kết nối đọc
  của kernel: không biết câu nào là câu tìm, thành trần chung cho mọi câu đọc — việc của lớp database mới (G1).
- Tìm theo khách bằng index ghép (`btree_gin`): cần extension, không có trên mọi máy Postgres.

Hệ quả: tìm chữ công khai khớp **theo từ** (tiền tố), không còn theo chuỗi con: `lat` vẫn tìm thấy `Đà Lạt`
nhưng `alat` thì không. Ghi ở changelog. Module khác cần tìm chữ công khai trên bảng lớn thì đưa `WordIndex` lên
kernel (sau G1) thay vì chép.
