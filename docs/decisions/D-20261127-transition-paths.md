## D-20261127-transition-paths · Đường cũ trong lúc chuyển: đếm bằng baseline, `#[deprecated]` ở bản chuyển xong — 2026-11-27 — agent quyết

Chủ đề: Kiến trúc module và kernel

Quyết định: Trong lúc chuyển module sang lớp mới của kernel (`db`, HTTP, manifest…), đường cũ của kernel **không** gắn
`#[deprecated]`. Số chỗ gọi còn lại trong repo được đếm bằng baseline của `cargo xtask check`, chỉ được giảm (như
`raw-database`, `module-contract-legacy`). Khi chỗ gọi cuối cùng trong repo đã chuyển, đường cũ được gắn
`#[deprecated(note = "use … instead")]` ngay ở bản phát hành đó: người viết module ngoài repo được báo trước một bản minor
(luật "đổi phá vỡ có `#[deprecated]` trước" của CLAUDE.md). Bản sau mới gỡ.

Lý do: gate chạy clippy với `-D warnings`, và luật không cho `#[allow(deprecated)]` để qua gate. Gắn `#[deprecated]` khi còn
hàng trăm chỗ gọi trong repo (≈25 module gọi `Outbox::record`, 29 module gọi `Database::read`) thì gate đỏ ngay, hoặc phải
tắt lint khắp nơi. Baseline giữ được cùng lời hứa chỉ giảm cho repo này. `#[deprecated]` vẫn là kênh báo cho người ngoài,
và đặt đúng lúc nó không làm đỏ gate của chính repo.

Đã cân nhắc:
- Gắn `#[deprecated]` ngay và `#[allow(deprecated)]` ở từng chỗ gọi: trái luật không tắt lint; che mất cảnh báo thật.
- Bỏ hẳn `#[deprecated]`, gỡ thẳng: người ngoài repo không được báo trước.

Thay: phần "đường cũ … giữ `#[deprecated]` tới khi mọi module chuyển" của D-20261126-kernel-data-layer (điểm 6)
