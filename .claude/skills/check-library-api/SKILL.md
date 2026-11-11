---
name: check-library-api
description: Tra API thật của một thư viện Rust ở đúng phiên bản Autoxide đang ghim, trước khi viết code dùng nó. Dùng mỗi khi sắp gọi một hàm, trait, macro hay feature flag của thư viện mà chưa từng dùng trong repo này; khi thêm hoặc nâng phiên bản một thư viện; khi trình biên dịch báo "no method named", "cannot find", "unresolved import", "trait bound not satisfied", "unrecognized feature" ở code gọi thư viện; hoặc khi định viết theo trí nhớ về SeaORM, axum, utoipa, sqlx, tokio, jsonwebtoken, argon2, tower-http. Các thư viện này mới hơn kiến thức có sẵn của model — viết theo trí nhớ là viết sai.
---

# Tra API thư viện đúng phiên bản

Phiên bản thư viện trong dự án này mới hơn kiến thức có sẵn của model (SeaORM 2,
utoipa 6, sqlx 0.9, tower-http 0.7, jsonwebtoken 11, argon2 0.6…). Cú pháp nhớ
được thường là của phiên bản cũ: trông đúng, không biên dịch được, hoặc tệ hơn là
biên dịch được nhưng hành vi đã đổi.

**Nguồn sự thật là mã nguồn của đúng phiên bản đang ghim, nằm sẵn trên máy** —
không phải trí nhớ, không phải bài viết trên mạng.

## 1. Biết đang dùng phiên bản nào

Thư viện đã có trong workspace:

```bash
cargo tree -p <crate> --depth 0        # in: <crate> v<phiên bản>
```

Chưa có (đang cân nhắc thêm):

```bash
cargo info <crate>                     # phiên bản mới nhất, phiên bản Rust tối thiểu, danh sách feature flag
```

## 2. Tìm thư mục mã nguồn

```bash
cargo fetch -q
dirname "$(cargo metadata --format-version 1 | jq -r '.packages[] | select(.name=="<crate>") | .manifest_path')"
```

Lệnh in ra đường dẫn dạng `~/.cargo/registry/src/index.crates.io-*/<crate>-<phiên bản>/`.
Nhiều phiên bản cùng tên thì `jq` in nhiều dòng — chọn dòng khớp bước 1.

## 3. Đọc đúng chỗ

Trong thư mục đó, theo thứ tự hữu ích:

| Xem                         | Để biết                                                         |
| --------------------------- | --------------------------------------------------------------- |
| `README.md`                 | Cách dùng cơ bản của phiên bản này                              |
| `CHANGELOG.md` (nếu có)     | Cái gì đổi so với phiên bản model còn nhớ — **đọc mục nâng cấp lớn** |
| `examples/`, `tests/`       | Code chạy được, đúng cú pháp hiện tại                           |
| `src/lib.rs`                | Cấu trúc module, những gì được export, ghi chú feature flag     |
| `Cargo.toml`                | Feature flag và phụ thuộc thật                                  |

Tìm một API cụ thể bằng tìm kiếm trong `src/`:

```bash
grep -rn "pub fn <tên>" <thư mục>/src
grep -rn "pub trait <Tên>" <thư mục>/src
grep -rn "pub struct <Tên>" <thư mục>/src
```

Đọc cả chữ ký hàm lẫn phần chú thích `///` phía trên — chú thích thường có ví dụ.
Macro thì nằm ở crate đi kèm (vd `sea-orm-macros`, `utoipa-gen`): tra crate đó.

Cần tài liệu dạng trang thì `cargo doc -p <crate> --no-deps` sinh ra
`target/doc/<crate>/`; nhưng đọc thẳng mã nguồn thường nhanh hơn.

## 4. Thêm thư viện

```bash
cargo add <crate> --features <a>,<b>
```

- **Không** viết `cargo add <crate>@1`: dạng có `@<số lớn>` kiểm feature flag theo
  một bản phát hành cũ và báo nhầm "unrecognized feature" cho feature có thật (đã gặp
  với `uuid` và feature `v7`, ngày 11/11/2026). Để cargo tự ghim bản mới nhất.
- Trong workspace, khai phiên bản ở `[workspace.dependencies]` của `Cargo.toml`
  gốc, crate con dùng `<crate>.workspace = true`.
- Thư viện mới phải được ghi vào `STACK.md`; đổi hoặc thêm thư viện là một quyết
  định — ghi bằng skill `record-decision`.

## 5. Để trình biên dịch xác nhận

Viết đoạn nhỏ nhất dùng API vừa tra, rồi:

```bash
cargo check
```

Xanh rồi mới viết tiếp phần dựa vào nó. Đừng viết vài trăm dòng theo một API chưa
được biên dịch lần nào.

Lỗi biên dịch của Rust thường nói thẳng cách sửa (`help: …`, `note: …`) — đọc hết
thông báo trước khi đoán.

## 6. Hai thư viện phải hợp nhau

Nhiều thư viện trong stack chỉ chạy với một dải phiên bản của thư viện khác:
`utoipa-axum` ↔ `axum`, `sea-orm` ↔ `sqlx`, `tower-http` ↔ `tower` / `http`.
Dấu hiệu lệch: lỗi kiểu "expected `X`, found `X`" (cùng tên, khác phiên bản).

```bash
cargo tree -d            # liệt kê thư viện đang có HAI phiên bản trong cây phụ thuộc
cargo tree -i <crate>    # ai đang kéo <crate> vào, ở phiên bản nào
```

Thấy trùng ở các thư viện nền (`axum`, `sqlx`, `http`, `tower`) thì hạ hoặc nâng
một bên cho khớp, đừng viết code chuyển đổi giữa hai phiên bản.

## Không làm

- Không dùng API vì "nhớ là có". Nhớ là điểm bắt đầu để tìm, không phải bằng chứng.
- Không tin đoạn mã trên mạng nếu chưa đối chiếu với mã nguồn trên máy: phần lớn
  viết cho phiên bản cũ.
- Không hạ phiên bản thư viện chỉ để khớp với cú pháp mình nhớ.
