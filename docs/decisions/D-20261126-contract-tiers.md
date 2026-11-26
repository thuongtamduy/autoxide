## D-20261126-contract-tiers · Ba hạng của phần công khai: ổn định, thử nghiệm (feature `unstable`), nội bộ (`autoxide-host`) — 2026-11-26 — agent quyết

Chủ đề: Kiến trúc module và kernel

Quyết định: Phần công khai của kernel và testkit chia ba hạng. **Ổn định**: đường `autoxide_kernel::<mô-đun>`, SemVer từ
1.0, `cargo semver-checks` kiểm với feature mặc định. **Thử nghiệm**: sau feature `unstable` (tắt mặc định) ở đường
`autoxide_kernel::unstable::<mô-đun>`, đổi được ở bản minor; module muốn dùng phải bật feature. **Nội bộ**: thứ chỉ binary
dùng chuyển sang crate `autoxide-host`, không phải hợp đồng. Danh sách xuất phát (mục 2.1 của tài liệu thiết kế): thử
nghiệm gồm hợp đồng bộ nối nhà cung cấp (`einvoice`, `zns`, `voice`, `sms`, `push`, `payment`), tính năng chưa chín
(`blocks`, `analytics`, `report`, `automation`, `custom_fields`, `sync`…) và phần đa khách mở rộng (`sandbox`,
`distribution`, `moves`, `domains`…); `bench_seed` sang testkit. Đường dẫn đổi thẳng trước 1.0 (Rust không cảnh báo
`#[deprecated]` trên `pub use`), ghi ở `### Breaking (Rust)`. Enum/struct công khai sẽ lớn thêm có `#[non_exhaustive]`
(ngoại lệ liệt kê có lý do); `#[deprecated]` luôn có `note` nêu cái thay thế; gỡ `TenantContext::legacy_default`.
Lên hạng ổn định là một quyết định mới. Hạng của module tách khỏi hạng của kernel: module ổn định
được dùng phần thử nghiệm bên trong nhưng `api` của nó không lộ kiểu thử nghiệm (gate kiểm); module thương mại và bộ nối mang
nhãn thử nghiệm lúc 1.0.

Lý do: 70 mô-đun đều công khai và đều bị coi là hợp đồng (`docs/COMPATIBILITY.md`), kể cả phần chỉ binary dùng (`backup`,
`transfer`, `logging` — 0 module dùng) và phần mới một người dùng. Hứa SemVer cho tất cả lúc 1.0 sẽ khoá những hình dạng chưa
chín. Feature + mô-đun riêng là cách quen thuộc của hệ Rust (`tokio_unstable`), người dùng thấy rõ mình dựa vào gì, và
semver-checks bỏ qua được mà không phải ẩn tài liệu.

Đã cân nhắc: `#[doc(hidden)]` cho phần thử nghiệm (ẩn luôn tài liệu); mỗi phần thử nghiệm một crate (nhiều crate, đổi
đường khi lên hạng vẫn là phá vỡ); giữ một hạng (khoá hình dạng non). Thiết kế đầy đủ, API mẫu và thứ tự PR: `docs/research/kernel-contract-1-0.md`.

Thay: phần "toàn bộ kernel và testkit là hợp đồng" của D-20261123-compatibility-policy
