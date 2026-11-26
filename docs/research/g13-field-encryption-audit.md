# Thiết kế G13: mã hoá mức trường cho dữ liệu cá nhân nhạy cảm, nhật ký kiểm toán khó sửa lén

Trạng thái: **thiết kế, chưa code.** Code sau khi PR 2–5 của G1 (hợp đồng kernel, `docs/research/kernel-contract-1-0.md`)
được merge, vì cả hai phần cắm vào lớp `db` mới (`#[derive(Record)]`, handle) và bước egress che dữ liệu (§14.1 của thiết
kế G1). Mục lộ trình: bước 71 G13 (D-20261126-release-1-0-readiness). Đầu vào: `docs/THREAT_MODEL.md`, ranh giới 3 và 5.

Văn bản pháp luật nêu dưới đây cần đối chiếu văn bản gốc / người có chuyên môn pháp lý.

## 1. Hiện trạng

| Có rồi | Ở đâu | Thiếu gì cho G13 |
| --- | --- | --- |
| Niêm phong secret: AES-256-GCM, AAD = mục đích + khách + dòng, `sealed:v1:<key_id>:…`, xoay khoá bằng `autoxide secrets reseal` | `crates/kernel/src/security/seal.rs` (D-257) | Chỉ cho secret mà module đọc lại (TOTP, khoá đối tác). Không có cho dữ liệu cá nhân, chưa có khoá theo khách, khoá chủ không tách ra ngoài máy chủ được |
| Khoá gốc `AUTOXIDE_SECRET_KEY`; thiếu thì kernel tự sinh và lưu ở `kernel_secrets` | `security/secret.rs` (D-113) | Khi khoá nằm trong database, bản dump chứa cả khoá: mã hoá không chống được kẻ lấy dump |
| Audit là chuỗi băm theo khách: `sequence`, `payload_hash`, `chain_hash`, đầu chuỗi ở `audit_chain`; `autoxide doctor` có bước `audit.chain`; xoá dữ liệu cá nhân vẫn giữ chuỗi | `modules/audit/src/chain.rs` (D-183, D-276) | Chuỗi không có khoá: ai ghi được database thì tính lại được cả chuỗi. Không neo ra ngoài: xoá đuôi cùng đầu chuỗi không để lại dấu |
| Phân loại trường `data_class = personal \| sensitive \| financial` (thiết kế) | G1 §14, §14.1 | Chỉ nói che khi đọc; chưa nói lưu thế nào |

## 2. Phần A — mã hoá mức trường

### 2.1 Phạm vi

Dữ liệu cá nhân nhạy cảm theo Luật Bảo vệ dữ liệu cá nhân (Luật 91/2025/QH15, hiệu lực 01/01/2026) và Nghị định
13/2023/NĐ-CP, Điều 2 khoản 4 (cần đối chiếu): số định danh và ảnh giấy tờ (`compliance` KYC), số tài khoản ngân hàng
(`ledger`, chi trả), dữ liệu sức khoẻ (`bookings`, `forms` nếu khách khai), vị trí chính xác, sinh trắc học. Danh sách cột
cụ thể lập ở PR đầu (rà theo `data_class = sensitive`), không viết cứng trong kernel.

Không mã hoá: cột dùng để lọc, sắp, nối, khoá ngoại; tiền (cần cộng, đối soát); `tenant_id`, `org_id`, ID. Cột vừa nhạy
cảm vừa cần tra đúng giá trị (số điện thoại, số CCCD) dùng **blind index** (§2.5).

### 2.2 Khoá: envelope encryption

- **KEK** (khoá mã hoá khoá): một trait mới của kernel `KeyProvider` — `wrap(dek) -> WrappedKey`, `unwrap(WrappedKey) ->
  Dek`, `current_kek_id()`. Cài đặt mặc định `LocalKeyProvider`: KEK = `SecretKey::derive("autoxide/kernel/fields/kek/v1")`
  từ khoá gốc. Nhà cung cấp ngoài (AWS KMS, Google Cloud KMS, HashiCorp Vault Transit) là module bộ nối, cắm vào bằng
  `context.provide::<dyn KeyProvider>`. Khoá chủ khi đó không bao giờ ở máy chủ Autoxide.
- **DEK** (khoá mã hoá dữ liệu): mỗi khách một DEK cho mỗi mục đích (`<module>.<cột>` hay nhóm cột), sinh ngẫu nhiên 256 bit,
  lưu **đã bọc** trong bảng mới của kernel `kernel_data_keys` (cấp khách, ở mọi placement, đi theo khách khi dời / tách /
  gộp): `id`, `tenant_id`, `purpose`, `wrapped_key`, `kek_id`, `created_at`, `retired_at`. DEK đã mở giữ trong bộ nhớ có hạn
  (LRU theo khách, `Zeroizing`), không ghi log, không vào bản dump bộ nhớ có chủ đích.
- Khách có database riêng mang DEK của mình trong chính database đó; KEK vẫn ở `KeyProvider`. Dump một placement không đủ
  để đọc dữ liệu.
- `doctor` cảnh báo (production: lỗi) khi có cột mã hoá mà khoá gốc nằm trong database (`kernel_secrets`) và
  `KeyProvider` là mặc định: mã hoá khi đó không chống được dump.

### 2.3 Định dạng giá trị

`enc:v1:<dek_id>:<base64url(nonce 96 bit || ciphertext || tag)>`, lưu trong cột `TEXT` (như nhau trên SQLite và Postgres).
AES-256-GCM qua `ring` (đã có, D-043). AAD = `table|column|tenant_id|primary key`: chép giá trị sang dòng, cột hay khách
khác thì không mở được (như `seal`). Định dạng `v1` là định dạng lưu trữ: đổi thì thêm `v2`, không sửa.

### 2.4 Cắm vào G1

- `#[derive(Record)]` nhận `#[record(encrypted)]` trên trường kiểu `Sensitive<T>` (`T`: `String`, `Option<String>`,
  `serde_json::Value`). Thiếu `encrypted` trên trường `data_class = sensitive` là lỗi biên dịch, trừ khi ghi rõ
  `#[record(plaintext = "<lý do>")]`. Gate mới `sensitive-plaintext` đếm các ngoại lệ đó.
- `Sensitive<T>` không cài `Display` / `Serialize`; `Debug` in `«sensitive»`. Lấy giá trị bằng `reveal(&principal)`: handle
  kiểm quyền lớp dữ liệu (`<module>.read_sensitive`, G1 §14.1), ghi audit "đọc dữ liệu nhạy cảm" qua đường riêng của kernel.
- Handle `db` mã hoá khi ghi, giải mã khi đọc (mở DEK theo khách của handle — khách đã được handle quyết, G1 §3.2). Không
  có đường ghi `Sensitive` bằng SQL viết tay: `db::raw` không nhận kiểu đó.
- Lọc / sắp theo trường mã hoá: không cho (lỗi biên dịch, cột `Col<R, Sensitive<_>>` không cài `eq`, `order_by`), trừ tra đúng
  giá trị qua blind index.

### 2.5 Tra đúng giá trị: blind index

Cột kèm `<cột>_bidx` = HMAC-SHA256(khoá blind index của khách + mục đích, giá trị đã chuẩn hoá), cắt 16 byte, base64url.
Khoá blind index là DEK riêng (`purpose = <module>.<cột>.bidx`). Chỉ hỗ trợ `eq` / `IN`. Rủi ro: lộ tần suất (hai dòng cùng
giá trị có cùng `bidx`). Chỉ dùng cho cột cần tra (số điện thoại đăng nhập, số CCCD chống trùng), ghi trong quyết định của
module.

### 2.6 Xoay khoá

- **Xoay KEK:** bọc lại mọi DEK bằng KEK mới (vài dòng mỗi khách, một transaction). `autoxide keys rotate-kek`.
- **Xoay DEK:** tạo DEK mới cho mục đích, đánh dấu cũ `retired_at`. Job nền `kernel.fields.rekey` mã hoá lại từng lô (keyset
  theo khoá chính, chạy được lại, idempotent nhờ `dek_id` trong giá trị), có tiến độ (`autoxide keys status`). Đọc được cả
  DEK cũ tới khi job xong; xong thì DEK cũ xoá bọc (giữ dòng để kiểm). Chạy như nhau trên hai database (chỉ SELECT / UPDATE theo
  khoá chính).
- **Lộ khoá:** sổ tay `docs/RUNBOOK.md` (G14) có bước: xoay KEK, xoay mọi DEK của khách bị ảnh hưởng, thu hồi bản sao lưu cũ.

### 2.7 Dữ liệu cũ và xoá dữ liệu

- Chuyển dần: giá trị không có tiền tố `enc:` là bản rõ cũ, đọc được trong thời gian chuyển; job `kernel.fields.rekey` mã
  hoá hết; `doctor` báo còn bao nhiêu dòng rõ.
- Xoá theo yêu cầu chủ thể dữ liệu vẫn xoá giá trị như hôm nay. Crypto-shredding (một DEK mỗi người, xoá DEK là xoá dữ liệu
  cả trong bản sao lưu) là bước sau, chỉ cho bảng lớn như hồ sơ KYC; ghi thành quyết định riêng khi làm.

### 2.8 Hiệu năng và test

AES-GCM nhanh hơn nhiều so với một lượt hỏi database; chi phí thật là mở DEK (một lần mỗi khách, có cache). Đo bằng
`docs/BENCHMARKS.md` trước và sau trên danh sách có trường mã hoá. Test trên cả hai database:

- ghi rồi đọc lại;
- chép giá trị sang dòng / khách khác thì không mở được;
- xoay DEK giữa chừng rồi chạy lại;
- dời khách sang placement khác mà vẫn đọc được;
- bản dump không có KEK thì không đọc được.

## 3. Phần B — nhật ký kiểm toán khó sửa lén

### 3.1 Mối đe doạ còn hở (threat model, ranh giới 5)

1. Người ghi được database (quản trị database, kẻ chiếm máy chủ database) sửa một dòng rồi **tính lại** `chain_hash` của
   mọi dòng sau và đầu chuỗi: chuỗi v1 không có khoá nên vẫn "nguyên vẹn".
2. Xoá đuôi chuỗi cùng lúc lùi đầu chuỗi: không để lại dấu.
3. Dựng lại cả chuỗi của một khách từ đầu.

### 3.2 Chuỗi có khoá (`audit-v2`)

`chain_hash_v2 = HMAC-SHA256(K_audit(tenant), "audit-v2" || chain_hash trước || các phần như v1)`. `K_audit` lấy từ
`KeyProvider` (§2.2, `purpose = audit.chain`), không lưu trong database khi khoá gốc ở ngoài. Dòng v1 giữ nguyên; dòng đầu
tiên của v2 nối vào `chain_hash` v1 cuối (điểm chuyển ghi ở `audit_chain.format`). `doctor` kiểm cả hai đoạn. Kẻ chỉ có
quyền ghi database không tính lại được v2. Mất `K_audit` thì mất khả năng **kiểm**, không mất dữ liệu: ghi rõ trong
RUNBOOK, cùng cách sao lưu khoá.

### 3.3 Neo ra ngoài (checkpoint có chữ ký)

- Job `audit.anchor` (mặc định mỗi 15 phút, mỗi khách có dòng mới) tạo checkpoint `{tenant, sequence, chain_hash, at}`, ký
  Ed25519 bằng khoá lấy từ `KeyProvider`, gửi qua trait mới `AuditAnchor` (module bộ nối): nơi lưu file có khoá ghi một lần
  (S3 Object Lock / tương đương), webhook tới hệ thống của khách, hay dịch vụ đóng dấu thời gian RFC 3161. Mặc định không
  bật (không có nơi ngoài nào là chắc chắn có); bật thì `doctor` kiểm checkpoint gần nhất khớp chuỗi.
- `autoxide audit verify --anchors <nơi lưu>` so chuỗi với mọi checkpoint: xoá đuôi hay dựng lại chuỗi trước một checkpoint
  đều lộ.
- Checkpoint không chứa dữ liệu cá nhân (chỉ số thứ tự và mã băm), nên gửi ra ngoài không vướng luật dữ liệu cá nhân.

### 3.4 Không đổi

Xoá dữ liệu cá nhân vẫn giữ chuỗi như D-183 (băm `payload_hash`, không băm dữ liệu). Audit vẫn chạy qua event, không trên
đường request. Ghi nối đuôi một dòng mỗi lần như v1: hai database như nhau.

## 4. Phối hợp với phiên "Hợp đồng kernel" (G1 / G2)

Cần chốt với G1 trước khi code (qua phiên điều phối):

1. Tên thuộc tính trên `#[derive(Record)]` (`encrypted`, `plaintext = "…"`) và kiểu `Sensitive<T>` đi vào bộ kiểu trường
   được nhận (G1 §3.1) — cần có từ PR 2 để module không đổi chữ ký sau.
2. Chỗ handle gọi codec mã hoá: sau khi quyết khách, trước khi dựng câu; `db::raw` cấm `Sensitive`.
3. `KeyProvider`, `AuditAnchor` thuộc phần công khai hạng nào (G1 §2) và nằm ở crate nào khi tách kernel (G1 §6).
4. `kernel_data_keys` là bảng cấp khách của kernel ở mọi placement, như `kernel_shares` (G1 §14).
5. Bước egress (G1 §14.1) đối xử `Sensitive` như `data_class = sensitive`: che nếu thiếu quyền, kể cả khi trường chưa mã hoá.

## 5. Thứ tự PR (sau G1 PR 2–5)

1. Kernel: `KeyProvider` + `LocalKeyProvider`, `kernel_data_keys`, codec `enc:v1`, `Sensitive<T>`, test hai database.
2. Kernel: job `kernel.fields.rekey`, lệnh `autoxide keys rotate-kek | rotate-dek | status`, bước `doctor`.
3. Module: `compliance` (KYC) và `ledger` (số tài khoản chi trả) chuyển cột sang `Sensitive`, kèm blind index nơi cần tra.
4. Audit: chuỗi `audit-v2` có khoá, điểm chuyển, `doctor` kiểm hai đoạn.
5. Audit: `AuditAnchor`, job `audit.anchor`, `autoxide audit verify --anchors`.
6. Tài liệu người dùng (EN + VI): cấu hình `KeyProvider`, sao lưu khoá, RUNBOOK lộ khoá.

Mỗi PR chạm kernel, bảo mật: kiểm độc lập trước khi merge (D-20261126-independent-review).
