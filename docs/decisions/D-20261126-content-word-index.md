## D-20261126-content-word-index · Tìm chữ công khai của `content`: chỉ mục đảo bằng bảng thường, giống nhau trên hai database, có trần thời gian câu lệnh — 2026-11-26 — agent quyết

Chủ đề: Vận hành và hiệu năng

Quyết định: tìm chữ trên bản công khai của `content` (danh sách bài `GET /public/posts?q=`, phần "theo chữ" của
`GET /public/search`, gợi ý tìm kiếm) không còn dùng `search_key LIKE '%q%'` (quét mọi bài còn lại sau bộ lọc, PT-15).
Thay bằng chỉ mục đảo `content_published_words` — bảng thường của module, giống hệt nhau trên SQLite và Postgres
(`modules/content/src/publishing/words.rs`):

- **Bảng**: một dòng mỗi từ của một bài đã đăng, `(tenant_id, entry_id, word, locale)`, khoá chính
  `(tenant_id, entry_id, word)`, index `(tenant_id, locale, word, entry_id)`. Cột `word` so sánh theo byte (Postgres
  `COLLATE "C"`, SQLite mặc định `BINARY`), để khoảng tiền tố đúng thứ tự điểm mã Unicode.
- **Từ** do Rust tách, không do database: `words(text)` = `search::fold` (bỏ dấu, chữ thường) rồi cắt ở mọi ký tự
  không phải chữ / số (`char::is_alphanumeric`), bỏ từ dài quá 64 ký tự, mỗi từ một lần, tối đa 4 000 từ và 64 KiB mỗi
  bài (chặn số dòng ghi mỗi lần đăng).
- **Ghi**: cùng transaction với bản đăng — đăng / đăng lại xoá dòng cũ của bài rồi chèn lại; gỡ / xoá bài xoá dòng
  của bài. Bảng có `tenant_id` nên xoá, xuất, dời, tách, gộp khách đi theo cơ chế chung của kernel (test dời khách
  kiểm bảng này). Migration `m0036_word_index` tạo bảng và tính từ cho bài đã đăng theo lô, có `down`.
- **Câu tìm**: `WordQuery::parse(q)` tách bằng cùng hàm, bỏ từ trùng, tối đa 8 từ; mọi từ phải có (AND, mỗi từ một
  `entry_id IN (…)` trên chỉ mục); từ từ 3 ký tự trở lên khớp **tiền tố** bằng khoảng `word >= w AND word < w_next`
  (`next_prefix`: tăng ký tự cuối lên ký tự kế tiếp, bỏ qua vùng surrogate, ký tự cuối là `char::MAX` thì tăng ký tự
  trước), từ 1–2 ký tự khớp nguyên từ. Câu không còn từ nào thì không có kết quả.
- **Hai bước**: (1) `matching_ids` lấy tối đa `MATCH_CAP` (1 000) ID khớp, mới đăng trước, trong khách, ngôn ngữ,
  loại, bỏ bài bị ẩn, chỉ bài ở các nút của website người gọi (`org_id` thuộc chuỗi nút) và bỏ bài `noindex` khi là
  gợi ý — câu duy nhất chạm chỉ mục; (2) câu thường như cũ (điều kiện "hiện trên website" đầy đủ `reading::on_site`
  với bài bị che ở nút gần hơn, phân loại, tác giả, keyset pagination) thêm `entry_id IN (…)`. Lọc nút ở bước (1): nếu
  không, 1 000 bài khớp mới hơn ở website khác của cùng khách lấp hết trần và website này không thấy gì.
- **Không SQL viết tay**: bước (1) dựng bằng bộ dựng câu của lớp `db` của kernel (`tenant.db().read()`, `Record`
  `PublishedHit` và `IndexedWord` chỉ để đọc; ghi vẫn qua entity SeaORM của module cho tới khi chuyển cả module), nên
  lọc khách do handle của kernel làm, cả trong subquery của từng từ.
- **Trần thời gian** (điểm còn lại của #548): `Find::within(2 s)` của kernel (D-20261127-db-read-within) — database
  dừng câu khi quá trần và khi người gọi thôi chờ; quá hạn → `503 content.public_search_timeout` (mã đã có).
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
Chỉ mục đảo đưa chi phí về theo số bài khớp, không theo số bài của khách; trần thời gian là chốt cuối khi từ khớp
quá nhiều. Bảng thường theo đúng luật đa khách (cột `tenant_id`, index bắt đầu bằng nó) và Luật số một (một cách
làm cho cả hai database), không cần ngoại lệ hay SQL viết tay.

Đã cân nhắc:

- (A) Chỉ mục tìm chữ của database — GIN trên `tsvector` (Postgres), bảng ảo FTS5 (SQLite), sau trait riêng (bản đầu
  của PR): bảng ảo FTS5 và các bảng phụ của nó không có `tenant_id`, nên phải thêm ngoại lệ cho bảng ảo vào kiểm đa
  khách của testkit; câu `@@` / `MATCH` phải viết tay (`db.sql`); hai cài đặt khác nhau theo database; `copy` phải bỏ
  bảng phụ. Bỏ vì trái "không nới luật" và tăng chỗ rẽ nhánh. Đổi lại (B) ghi nhiều dòng hơn mỗi lần đăng (tới 4 000
  dòng, đã có trần) và bảng lớn hơn GIN.
- Tách từ bằng database (`to_tsvector`, `unicode61`): bộ tách của hai bên khác nhau (email, URL, số thực, gạch
  nối…), hai database cho kết quả khác nhau.
- Khoảng tiền tố bằng `LIKE 'w%'`: trên Postgres chỉ dùng index khi cột có collation `C` hay index `text_pattern_ops`;
  với cột `COLLATE "C"` thì khoảng `>=` / `<` rõ hơn và chạy như nhau trên hai bên.
- Lọc website ở bước (2) (bản đầu của PR): website khác của cùng khách lấp trần `MATCH_CAP`.
- Trần ở tầng task (`tokio::time::timeout`): câu còn chạy nốt ở nền sau khi trả lỗi, không phải trần thật.

Hệ quả: tìm chữ công khai khớp **theo từ** (tiền tố), không còn theo chuỗi con: `lat` vẫn tìm thấy `Đà Lạt`
nhưng `alat` thì không. Ghi ở changelog. Module khác cần tìm chữ công khai trên bảng lớn thì đưa mẫu chỉ mục đảo lên
kernel thay vì chép.
