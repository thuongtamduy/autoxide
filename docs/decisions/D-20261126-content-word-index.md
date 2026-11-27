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
- **Các bước của một lần tìm** (`matching_ids`). Kết quả luôn là các bài mới đăng nhất khớp mọi từ, tối đa
  `MATCH_CAP` (1 000), không cắt theo số bài đã duyệt; hết hạn trước khi xong thì trả `503`, không trả thiếu. Mỗi câu
  đọc có trần số dòng, để từ rất phổ biến không làm một câu đọc hết chỉ mục của khách (hai lần review của #573):
  1. Mỗi từ: ID bài có từ đó, tối đa `TERM_ROWS_MAX` (50 000) dòng chỉ mục (index `(tenant_id, locale, word,
     entry_id)`), giao dần **trong bộ nhớ** (tối đa chừng 800 KB một từ). Từ không có dòng nào, hay giao rỗng: không
     có kết quả. Từ vượt trần là "phổ biến".
  2. Còn ít ứng viên (tối đa `DIRECT_MAX`, 2 000): kiểm từ phổ biến bằng khoá chính `(tenant_id, entry_id, word)` trên
     chính các bài đó (danh sách `IN` tối đa 500 ID mỗi câu), rồi lấy bài theo bộ lọc, mới đăng trước.
  3. Nhiều ứng viên, hay mọi từ đều phổ biến: duyệt bài theo bộ lọc, mới đăng trước, từng trang 1 000 bài (keyset theo
     ngày đăng và ID, index mới `content_published_newest_idx (tenant_id, locale, published_at, entry_id)` — index sẵn
     có bắt đầu bằng `kind`, không dùng được khi tìm mọi loại), giữ bài thuộc tập ứng viên (kiểm trong bộ nhớ, không
     tốn câu) và có các từ phổ biến (khoá chính), tới khi đủ `MATCH_CAP` bài hay hết bài.

  Câu dò bằng khoá chính **không lọc `locale`** (mỗi bài chỉ một ngôn ngữ, ứng viên đã đúng ngôn ngữ): có `locale`,
  Postgres có lúc chọn index `(tenant_id, locale, word, entry_id)` và quét cả khoảng từ — đo được tới 640 ms một câu
  với 50 000 bài; không có `locale` thì chỉ khoá chính dùng được.

  Lọc bài (website / ngôn ngữ / loại / bị ẩn / `noindex` / chuyên mục / tác giả) trước khi cắt trần: nếu không, 1 000
  bài khớp mới hơn ở website hay chuyên mục khác lấp hết trần. Chuyên mục và tác giả là subquery trên
  `content_entry_terms` / `content_entry_authors` (index `(tenant_id, term_id | author_id, entry_id)`). Sau bước này,
  câu thường như cũ (điều kiện "hiện trên website" đầy đủ `reading::on_site` với bài bị che ở nút gần hơn, keyset
  pagination) thêm `entry_id IN (…)`. Mọi câu chung một hạn 2 s. Test dùng trần nhỏ (`#[cfg(test)]`: 2 000 / 200) để
  cả ba đường đi chạy với ít dữ liệu.

  Số đo (2026-11-27, máy phát triển 4 nhân, bản release, hằng số thật; một khách 60 000 bài: `ga` ở mọi bài — từ phổ
  biến; `pho` ở 45 000 bài cũ nhất; `hai` ở 25 000 bài cũ nhất; `mot` ở 25 000 bài mới nhất và 3 bài cũ nhất; `hiem`
  ở 3 bài; thời gian của `matching_ids`; Postgres đo cả trước và sau `ANALYZE`):

  | Câu tìm        | Kết quả | Đường đi                                   | SQLite      | Postgres 16  |
  | -------------- | ------- | ------------------------------------------ | ----------- | ------------ |
  | `hiem`         | 3       | bước 2                                     | 12–13 ms    | 4–26 ms      |
  | `pho hiem`     | 3       | bước 2                                     | 70–96 ms    | 33–52 ms     |
  | `ga`           | 1 000   | bước 3, một trang                          | 68–98 ms    | 32–63 ms     |
  | `ga pho`       | 1 000   | bước 3, bỏ qua 15 000 bài mới không khớp    | 208–234 ms  | 223–268 ms   |
  | `ga hai`       | 1 000   | bước 3, bỏ qua 35 000 bài mới không khớp    | 338–341 ms  | 796–864 ms   |
  | `hai mot`      | 3       | bước 2 (giao trong bộ nhớ)                 | 82–87 ms    | 32–46 ms     |
  | `pho mot`      | 1 000   | bước 3 (nhiều ứng viên)                    | 182–190 ms  | 220–250 ms   |
  | `khongco`, `ga khongco` | 0 | bước 1 dừng sớm                       | 0,4–67 ms   | 1–40 ms      |

  Bước 1 đọc mọi từ với trần nhỏ (`DIRECT_MAX`) trước: câu tiếng Việt tự nhiên nhiều từ phổ biến ("của là và các
  những…") có một từ hiếm thì không đọc tập lớn nào (review thứ ba). Số đo câu nhiều từ (cùng máy, 60 000 bài; `cua`
  `la` `va` `cac` `nhung` `ga` ở mọi bài, `dalat` ở 1/20 số bài, các từ khác như trên):

  | Câu tìm                               | Kết quả | SQLite      | Postgres 16 (trước / sau `ANALYZE`) |
  | ------------------------------------- | ------- | ----------- | ----------------------------------- |
  | `cua la va hiem`                      | 3       | 19–22 ms    | 25 / 10 ms                          |
  | `cua la va cac nhung hiem`            | 3       | 23 ms       | 30 / 13 ms                          |
  | `cua la va cac nhung ga pho hiem`     | 3       | 29 ms       | 41 / 23 ms                          |
  | `cua la va cac`                       | 1 000   | 273–280 ms  | 381 / 95 ms                         |
  | `cua la va dalat`                     | 1 000   | 372–417 ms  | 404 / 374 ms                        |
  | `cua la va cac nhung ga dalat hai`    | 1 000   | 593–615 ms  | 222 / 204 ms                        |
  | `cua la va cac nhung ga pho mot`      | 1 000   | 581–666 ms  | 408 / 359 ms                        |
  | `cua la va khongco`                   | 0       | 8 ms        | 5 / 5 ms                            |

  Bài bị bài cùng đường dẫn ở nút gần hơn che (`on_site`) vẫn được tính vào trần 1 000 trước khi bước sau bỏ nó:
  trường hợp hiếm (hai nút cùng đường dẫn), kết quả có thể ít hơn 1 000 bài một chút, không sai.

  Rủi ro còn lại: câu có từ rất phổ biến (hơn 50 000 bài) mà bài khớp mọi từ chỉ nằm sâu trong bài cũ thì bước 3 duyệt
  nhiều trang, mỗi trang thêm câu dò khoá chính; trên khách rất lớn có thể hết hạn (`503`, "thử từ cụ thể hơn").
  Không bao giờ trả kết quả sai hay thiếu.
- **Không SQL viết tay**: bước (1) dựng bằng bộ dựng câu của lớp `db` của kernel (`tenant.db().read()`, `Record`
  `PublishedHit` và `IndexedWord` chỉ để đọc; ghi vẫn qua entity SeaORM của module cho tới khi chuyển cả module), nên
  lọc khách do handle của kernel làm, cả trong subquery của từng từ.
- **Trần thời gian** (điểm còn lại của #548): `Find::within` của kernel (D-20261127-db-read-within), mọi câu của một
  lần tìm chung hạn 2 s — database dừng câu khi quá hạn và khi người gọi thôi chờ; quá hạn → `503
  content.public_search_timeout` (mã đã có).
- **Chốt tìm công khai** (review thứ ba của #573): một trần chung cho tìm và gợi ý **trên mỗi database** (mỗi
  placement một pool đọc): `min(AUTOXIDE_CONTENT_PUBLIC_SEARCH_CONCURRENCY, pool đọc − 2)`, ít nhất 2; gợi ý được một
  phần tư (ít nhất 1), tìm kiếm (`GET /public/search`, danh sách bài có `q`) phần còn lại — hai bên không lấy chỗ của
  nhau, và cộng lại luôn để dành ít nhất 2 kết nối đọc cho phần còn lại của ứng dụng (pool từ 4 trở lên; pool mặc định
  10 → 6 tìm + 2 gợi ý; pool của placement mặc định 4 → 1 + 1). Trần tự hạ theo pool thật (`Database::read_pool_size`
  của kernel), không cần kiểm lúc khởi động. Tìm kiếm còn đếm theo địa chỉ (`…_PER_ADDRESS_PER_MINUTE`); gợi ý không
  biết địa chỉ người đọc nên không đếm theo địa chỉ. Hết chỗ: tìm trả 429 `content.public_search_busy`, gợi ý bỏ nguồn
  `content`. Rủi ro còn lại: một địa chỉ vẫn có thể giữ hết phần chỗ của gợi ý (gợi ý bài viết tạm vắng; tìm kiếm và
  các câu đọc khác không ảnh hưởng). Việc sau: truyền địa chỉ người đọc tới nguồn gợi ý qua kernel (`SuggestSource`).
  Hạn 2 s tính cả thời gian chờ kết nối của pool (`within` của kernel), nên một lượt không kéo dài tới thời gian chờ
  chung của pool.
- **Ghi**: mỗi câu `INSERT` tối đa 200 dòng (800 tham số bind), cả khi đăng lẫn trong backfill, dưới trần của SQLite
  (999 ở bản cũ) và Postgres (65 535). Migration xoá bảng nếu còn từ lần chạy hỏng trước rồi dựng lại (dữ liệu dẫn
  xuất), nên chạy lại được.
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
nhưng `alat` thì không. Chữ viết liền không có khoảng trắng (tiếng Trung, Nhật, Thái…) thành một "từ" dài: đoạn dài
quá 64 ký tự không vào chỉ mục và không tìm được; tìm được bằng phần đầu của đoạn (tiền tố). Ghi ở changelog. Module khác cần tìm chữ công khai trên bảng lớn thì đưa mẫu chỉ mục đảo lên
kernel thay vì chép.
