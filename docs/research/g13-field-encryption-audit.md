# Thiết kế G13: mã hoá mức trường cho dữ liệu cá nhân nhạy cảm, nhật ký kiểm toán khó sửa lén

Trạng thái: **thiết kế, chưa code.** Code sau khi PR 2–5 của G1 (hợp đồng kernel, `docs/research/kernel-contract-1-0.md`)
được merge, vì cả hai phần cắm vào lớp `db` mới (`#[derive(Record)]`, handle) và bước egress che dữ liệu (§14.1 của thiết
kế G1). Mục lộ trình: bước 71 G13 (D-20261126-release-1-0-readiness). Đầu vào: `docs/THREAT_MODEL.md`, ranh giới 3 và 5.

Văn bản pháp luật nêu dưới đây cần đối chiếu văn bản gốc / người có chuyên môn pháp lý.

## 1. Hiện trạng

| Có rồi | Ở đâu | Thiếu gì cho G13 |
| --- | --- | --- |
| Niêm phong secret: AES-256-GCM, AAD = mục đích + khách + dòng, `sealed:v1:<key_id>:…`, xoay key bằng `autoxide secrets reseal` | `crates/kernel/src/security/seal.rs` (D-257) | Chỉ cho secret mà module đọc lại (TOTP, key của đối tác). Không có cho dữ liệu cá nhân, chưa có key theo khách, key gốc không tách ra ngoài máy chủ được |
| Key gốc `AUTOXIDE_SECRET_KEY`; thiếu thì kernel tự sinh và lưu ở `kernel_secrets` | `security/secret.rs` (D-113) | Key gốc nằm trong database thì bản dump chứa cả key gốc (xem §2.2 về key store, nơi giữ tenant key) |
| Audit là chuỗi băm theo khách: `sequence`, `payload_hash`, `chain_hash`, đầu chuỗi ở `audit_chain`; `autoxide doctor` có bước `audit.chain`; xoá dữ liệu cá nhân vẫn giữ chuỗi | `modules/audit/src/chain.rs` (D-183, D-276) | Chuỗi không có key: ai ghi được database thì tính lại được cả chuỗi. Không neo ra ngoài: xoá đuôi cùng đầu chuỗi không để lại dấu |
| Phân loại trường `data_class = personal \| sensitive \| financial` (thiết kế) | G1 §14, §14.1 | Chỉ nói che khi đọc; chưa nói lưu thế nào |

## 2. Phần A — mã hoá mức trường

### 2.1 Phạm vi

Dữ liệu cá nhân nhạy cảm theo Luật Bảo vệ dữ liệu cá nhân (Luật 91/2025/QH15, hiệu lực 01/01/2026) và Nghị định
13/2023/NĐ-CP, Điều 2 khoản 4 (cần đối chiếu): số định danh (`compliance` KYC), số tài khoản ngân hàng (`ledger`, chi
trả), dữ liệu sức khoẻ (`bookings`, `forms` nếu khách khai), vị trí chính xác, sinh trắc học dạng giá trị. `Sensitive<T>`
chỉ nhận `String` và `serde_json::Value` (§14.2 a): **nội dung file** (ảnh giấy tờ, ảnh sinh trắc) nằm ngoài thiết kế này,
xử lý ở tầng lưu file. Danh sách cột cụ thể lập ở PR đầu (rà theo `data_class = sensitive`), không viết cứng trong kernel.

Không mã hoá: cột dùng để lọc, sắp, nối, foreign key; tiền (cần cộng, đối soát); `tenant_id`, `org_id`, ID. Cột vừa nhạy
cảm vừa cần tra đúng giá trị (số điện thoại, số CCCD) dùng **blind index** (§2.2 dưới đây, §14.2 g).

### 2.2 Cách ghép vào lớp `db`: nguồn duy nhất là §14.2 của hợp đồng kernel

Mọi chi tiết về cách mã hoá mức trường ghép vào lớp `db` và vòng đời key nằm ở **§14.2 của
`docs/research/kernel-contract-1-0.md`** (đã qua rà độc lập, #580). File này không nhắc lại để hai nơi không lệch nhau;
chỗ nào ở đây có vẻ khác §14.2 thì §14.2 đúng. §14.2 chốt:

- kiểu `db::Sensitive<T, C>` và `db::BlindIndex<T>`; **kiểu** quyết định việc mã hoá, không có thuộc tính `encrypted`;
  ngoại lệ `#[record(plaintext = "<lý do>")]` đếm bằng gate (§14.2 a);
- key ba tầng: KEK của bản cài → tenant key mỗi khách (trong KMS, hay trong key store `AUTOXIDE_KEY_STORE` ngoài database
  và ngoài backup của database) → DEK mỗi (khách, purpose) bọc bằng tenant key, lưu trong `kernel_data_keys`; thứ tự tạo
  key không bao giờ ghi đè; huỷ tenant key khi xoá khách (§14.2 b);
- đọc không giải mã: `Sensitive::reveal(handle, principal)` kiểm quyền tại nút của dòng; audit ghi đồng bộ ở bước egress,
  lỗi thì không đưa dữ liệu ra (§14.2 c, d);
- ghi: codec trong handle, key nạp trước transaction (§14.2 e);
- trạng thái cột chỉ tiến (`plaintext` → `migrating` → `encrypted`), không có đường quay về bản rõ (§14.2 f);
- AAD, định dạng và blind index (§14.2 g), xoay key (§14.2 h), giá trị đã `reveal` không lưu rõ ở chỗ khác (§14.2 i),
  khách: dời, tách, gộp, xoá, xuất, khôi phục (§14.2 j), hạng và crate (§14.2 l).

Phần riêng của file này, không có ở §14.2:

- **`KeyProvider`** là trait của kernel (thử nghiệm, `autoxide_kernel::unstable::security`, §14.2 l); nhà cung cấp ngoài (AWS
  KMS, Google Cloud KMS, HashiCorp Vault Transit) là module bộ nối cắm vào bằng `context.provide::<dyn KeyProvider>`, key
  gốc khi đó không bao giờ ở máy chủ Autoxide. Cài đặt mặc định `LocalKeyProvider` và key store của nó: §14.2 b.
- **Mối đe doạ mà mã hoá chặn được**: kẻ chỉ có bản dump database (hay backup của database) không đọc được trường
  `Sensitive`, vì tenant key ở key store (hay KMS), không ở database. Kẻ có **cả** dump lẫn bản sao key store (và key gốc
  bọc nó), hay chiếm được máy chủ đang chạy, thì đọc được: mã hoá mức trường không chặn trường hợp đó. `doctor` cảnh báo
  (production: lỗi) khi key store nằm cùng ổ, cùng đường backup với database, hay khi key gốc bọc key store nằm trong
  database (`kernel_secrets`) mà `KeyProvider` là mặc định.
- **Định dạng lưu trữ** `enc:v1:<dek_id>:<base64url(nonce 96 bit || ciphertext || tag)>`, cột `TEXT` (như nhau trên SQLite và
  Postgres), AES-256-GCM qua `ring` (đã có, D-043). `v1` là định dạng lưu trữ: đổi thì thêm `v2`, không sửa. AAD: §14.2 g.
  Bản mã vẫn để lộ **độ dài** của giá trị và việc có / không có giá trị (NULL), §14.2 g; tài liệu người dùng ghi rõ.
- **Blind index** chỉ cho cột thật sự cần tra đúng giá trị (số điện thoại đăng nhập, số CCCD chống trùng), ghi trong quyết
  định của module; rủi ro lộ tần suất (hai dòng cùng giá trị có cùng chỉ mục). Cách tính và xoay: §14.2 g, h.
- **Lộ key:** sổ tay `docs/RUNBOOK.md` (G14) có bước theo từng tầng: xoay KEK (bọc lại tenant key), xoay tenant key của khách
  bị ảnh hưởng (bọc lại DEK), xoay DEK của các purpose thường (mã hoá lại bằng job `kernel.fields.rekey`), thu hồi bản sao
  key store và backup cũ. **Ngoại lệ**: DEK của `kernel.audit` và mọi purpose nằm trong chuỗi băm của audit, cùng key của
  chuỗi audit (§3.2), chỉ chuyển sang chế độ chỉ giải mã / chỉ kiểm và dùng key mới cho dòng mới — không mã hoá lại, vì mã
  hoá lại đổi bản mã mà chuỗi đã băm (§14.2 h).
- **Crypto-shredding theo từng khách đã có**: xoá khách huỷ tenant key (§14.2 b, j; D-20261126-tenant-delete-key-shred), dữ
  liệu `Sensitive` của khách đó trong mọi backup không mở được nữa. **Theo từng người** (một DEK mỗi chủ thể dữ liệu, xoá DEK
  là xoá dữ liệu của người đó cả trong backup) là bước sau, chỉ cho bảng lớn như hồ sơ KYC; ghi thành quyết định riêng khi
  làm. Xoá theo yêu cầu chủ thể dữ liệu vẫn xoá giá trị như hôm nay.

### 2.3 Hiệu năng và test

AES-GCM nhanh hơn nhiều so với một lượt hỏi database; chi phí thật là mở DEK (một lần mỗi khách, có cache). Đo bằng
`docs/BENCHMARKS.md` trước và sau trên danh sách có trường mã hoá. Test trên cả hai database:

- ghi rồi đọc lại;
- chép giá trị sang dòng / khách khác thì không mở được;
- xoay DEK giữa chừng rồi chạy lại;
- dời khách sang placement khác mà vẫn đọc được; tách và gộp khách (blind index tính lại bằng key của khách đích, §14.2 g);
- chỉ có bản dump database, không có key store, thì không đọc được;
- database đã ghi nhận tenant key mà key store thiếu file hay file không khớp dấu vân tay: lỗi `db.tenant_key_missing`, không
  sinh key mới;
- `autoxide keys finalize <purpose>` chỉ chuyển sang `encrypted` khi không còn dòng rõ và không record nào khai `plaintext`;
- nâng cấp cuốn chiếu: bản cũ còn chạy ghi dòng rõ sau `finalize` thì đọc báo `db.field_not_encrypted`, `keys rekey` sửa được.

## 3. Phần B — nhật ký kiểm toán khó sửa lén

### 3.1 Mối đe doạ còn hở (threat model, ranh giới 5)

1. Người ghi được database (quản trị database, kẻ chiếm máy chủ database) sửa một dòng rồi **tính lại** `chain_hash` của
   mọi dòng sau và đầu chuỗi: chuỗi v1 không có key nên vẫn "nguyên vẹn".
2. Xoá đuôi chuỗi cùng lúc lùi đầu chuỗi: không để lại dấu.
3. Dựng lại cả chuỗi của một khách từ đầu.

### 3.2 Chuỗi có key (`audit-v2`)

- `chain_hash_v2 = HMAC-SHA256(K_audit, "audit-v2" || chain_hash trước || các phần như v1)`.
- **`K_audit`** là một mục của `kernel_data_keys` loại riêng (`kind = chain`, purpose `kernel.audit_chain` — khác purpose
  `kernel.audit` của §14.2 h), **bọc bằng KEK** (tầng nền tảng), không bằng tenant key: xoá khách (huỷ tenant key) không làm
  mất khả năng kiểm phần chuỗi audit mà luật buộc giữ lại. Không lưu ở dạng chưa bọc. Mỗi dòng v2 ghi id của `K_audit` đã
  dùng; xoay `K_audit` thì key cũ giữ lại để **kiểm** (không ký dòng mới).
- Dòng v1 giữ nguyên; dòng đầu tiên của v2 nối vào `chain_hash` v1 cuối (điểm chuyển ghi ở `audit_chain.format`). `doctor`
  kiểm cả hai đoạn. Kẻ chỉ có quyền ghi database không tính lại được v2. Mất `K_audit` (hay KEK bọc nó) thì mất khả năng
  **kiểm**, không mất dữ liệu: ghi rõ trong RUNBOOK, cùng cách backup key.

### 3.3 Neo ra ngoài (checkpoint có chữ ký)

- Job `audit.anchor` (mặc định mỗi 15 phút, mỗi khách có dòng mới) tạo checkpoint `{tenant, sequence, chain_hash, at}`, ký
  Ed25519. Key ký lưu cùng cách với `K_audit` (mục `kernel_data_keys` loại `sign`, bọc bằng KEK); `KeyProvider` không thêm
  thao tác ký. Checkpoint gửi qua trait `AuditAnchor` (thử nghiệm, `autoxide_kernel::unstable::security`, §14.2 l; module bộ
  nối): nơi lưu file ghi một lần (S3 Object Lock / tương đương), webhook tới hệ thống của khách, hay dịch vụ đóng dấu thời
  gian RFC 3161. Mặc định không bật (không có nơi ngoài nào là chắc chắn có); bật thì `doctor` kiểm checkpoint gần nhất
  khớp chuỗi.
- `autoxide audit verify --anchors <nơi lưu>` so chuỗi với mọi checkpoint: xoá đuôi hay dựng lại chuỗi trước một checkpoint
  đều lộ.
- Checkpoint không chứa dữ liệu cá nhân (chỉ số thứ tự và mã băm), nên gửi ra ngoài không vướng luật dữ liệu cá nhân.

### 3.4 Ghi dòng audit và niêm phong chuỗi

- **Dòng audit ghi đồng bộ, fail-closed** như §14.2 c: `reveal` ghi ở bước egress trước khi gửi response (lỗi thì
  `kernel.audit_unavailable`, không đưa dữ liệu ra); `reveal` trên `TenantTx` ghi trong chính transaction đó.
- **Việc nối vào chuỗi HMAC theo khách do một job niêm phong chạy sau**: job đọc các dòng chưa niêm phong của khách theo thứ
  tự id, tính `chain_hash_v2`, ghi vào dòng và đánh dấu đã niêm phong, cập nhật đầu chuỗi. Đầu chuỗi vì vậy không thành
  điểm tuần tự trên đường request. Dòng chưa niêm phong vẫn đọc được; `doctor` báo dòng chờ niêm phong quá lâu.
- Xoá dữ liệu cá nhân vẫn giữ chuỗi như D-183 (băm `payload_hash`, không băm dữ liệu). Ghi và niêm phong chỉ dùng
  INSERT / UPDATE theo primary key: hai database như nhau.

## 4. Phối hợp với phiên "Hợp đồng kernel" (G1 / G2)

Các điểm cần chốt với G1 (tên kiểu và thuộc tính, chỗ handle gọi codec, `db.sql` không nhận `Sensitive`, `kernel_data_keys`
là bảng cấp khách của kernel, egress đối xử `Sensitive` như `data_class = sensitive`, hạng và crate của `KeyProvider` và
`AuditAnchor`) đã chốt ở §14.2 của `docs/research/kernel-contract-1-0.md` (hạng và crate: §14.2 l).

## 5. Thứ tự PR (sau G1 PR 2–5)

Trước PR 1: gate `principal-constructors` có baseline rỗng (§14.2 d).

1. Kernel, theo §14.2: `KeyProvider` + `LocalKeyProvider` và key store, `kernel_data_keys`, `kernel_field_states`, codec
   `enc:v1`, `Sensitive<T, C>` / `BlindIndex<T>`, `on_conflict_do_nothing_where`, đổi `Col<R, T>` thành `Col<R, T, K>`; lệnh
   `autoxide keys init`; test hai database.
2. Kernel: job `kernel.fields.rekey`, lệnh `autoxide keys rotate-kek | rekey | finalize | status`, bước `doctor`.
3. Module: `compliance` (KYC) và `ledger` (số tài khoản chi trả) chuyển cột sang `Sensitive`, kèm blind index nơi cần tra.
   Phụ thuộc: hai module này chuyển cụm sang lớp `db` (bỏ thư viện bọc) trước (§14.2 e).
4. Audit: chuỗi `audit-v2` có key (`K_audit`, job niêm phong), điểm chuyển, `doctor` kiểm hai đoạn.
5. Audit: `AuditAnchor`, key ký, job `audit.anchor`, `autoxide audit verify --anchors`.
6. Tài liệu người dùng (EN + VI): cấu hình `KeyProvider`, key store và cách backup, RUNBOOK lộ key.

Mỗi PR chạm kernel, bảo mật: kiểm độc lập trước khi merge (D-20261126-independent-review).
