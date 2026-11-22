## D-20261122-module-test-fakes · Bản giả của trait module nằm ở `api::testing` của module đó, sau feature `testing` — 2026-11-22 — agent quyết

Chủ đề: Kiến trúc module và kernel

Quyết định (bước 70 P10, trải nghiệm người viết module):

- Bản giả của một trait mà module **công bố** (vd `Ledger` của `ledger`) nằm ở chính module đó:
  `autoxide_<tên>::api::testing`, chỉ biên dịch khi bật feature `testing`. Module khác dùng bằng
  `autoxide-<tên> = { workspace = true, features = ["testing"] }` trong `[dev-dependencies]` (bản build thật không
  mang bản giả). Bản giả chỉ dùng kiểu của `api` (luật `api-self-contained` giữ nguyên) và không `unwrap` (code ở
  `src/`).
- Bản giả đầu tiên: `autoxide_ledger::api::testing::FakeLedger` + `FakeLedgerModule` — bút toán chống trùng theo
  `reference` và phải cân, tài khoản suy từ khoá với bên thường, số dư theo bên thường, lượt thu / hoàn chống trùng,
  cài đặt dòng tiền "tự thu", phát `ledger.payment_succeeded` đúng đường outbox thật (`succeed`, `succeed_with`,
  `record_payment_succeeded`), giả lập sổ từ chối (`fail_next_posts`). Hàm chưa giả lập (kỳ, tạm giữ, chi trả, sổ
  nhật ký) trả `internal_error` nêu tên hàm. Sáu bản chép tay ở test của `orders`, `shipping`, `customers`, `plans`,
  `bookings`, `inventory` (~1.300 dòng) thay bằng bản này; hành vi riêng của từng bản (đọc lượt hoàn dạng bộ ba ở
  `orders`, thân event không qua lượt thu ở `plans`) giữ lại thành vài dòng trong test của module đó.
- Tên theo kiểu của testkit (`FakePaymentGateway`, `FakeTextModel`): `Fake<Trait>`; module giả bọc nó là
  `Fake<Trait>Module`.

Lý do: testkit không được biết module nào (luật `dependency`, D-031), còn trait thì là hợp đồng của module — bản
giả đi cùng hợp đồng thì đổi trait là sửa bản giả trong cùng crate, và người viết module ngoài repo cũng dùng được
(phần công khai của `api`). Sáu bản chép đã lệch nhau (hạn lượt thu 15 / 30 phút, có / không chống trùng lượt hoàn,
số dư theo bên thường / luôn Nợ − Có).

Đã cân nhắc: đặt ở `autoxide-testkit` — phải cho testkit phụ thuộc `autoxide-ledger`, phá luật chiều phụ thuộc và
bắt mọi test của mọi crate biên dịch thêm `ledger`; một crate `autoxide-ledger-testing` riêng — thêm một crate cho
mỗi module có trait, và crate đó chỉ thấy `api` công khai giống hệt feature; dùng `ledger` thật trong test của
module khác — cần cổng thanh toán giả, sổ, kỳ và nhiều bảng chỉ để thử luồng của module đang test, chậm hơn nhiều.
