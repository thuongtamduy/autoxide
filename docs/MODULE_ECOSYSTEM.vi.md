# Hệ sinh thái module

[English](MODULE_ECOSYSTEM.md) · **Tiếng Việt**

Core của Autoxide nhỏ và trung lập; tính năng nghiệp vụ nằm trong module. Module có thể ở repo này (module chính thức)
hoặc ở **repo riêng của bạn**, build dựa trên hợp đồng ổn định ([`docs/COMPATIBILITY.vi.md`](COMPATIBILITY.vi.md)). Trang này
nói cách đặt tên, mô tả, đăng và liệt kê một module để người khác tìm được và biết nó chạy được. Cách viết một module:
[`docs/MODULE_GUIDE.vi.md`](MODULE_GUIDE.vi.md) và [`docs/guides/writing-a-module.vi.md`](guides/writing-a-module.vi.md).

## Đặt tên

- Crate đặt tên **`autoxide-<tên>`** (chữ thường, nối bằng gạch ngang), ví dụ `autoxide-bookmarks`. Bảng bắt đầu bằng
  `<tên>_` ([luật sở hữu bảng](MODULE_GUIDE.vi.md)), permission bằng `<tên>.`, mã lỗi bằng `<tên>.`.
- Tên các module chính thức trong repo này được giữ chỗ. Với module của bạn, kiểm <https://crates.io> và
  [danh sách cộng đồng](#danh-sách-cộng-đồng) trước: chọn tên nói rõ module làm gì (`autoxide-shipping-ghn`, không phải
  `autoxide-best`). Nếu tên đã có module cùng việc, cân nhắc đóng góp vào đó.
- Tên có tiền tố `autoxide-` nói "module này cắm vào Autoxide". Nó không nói "do dự án Autoxide làm hay bảo trợ": xem
  [`TRADEMARK.vi.md`](../TRADEMARK.vi.md).

## Giúp người khác tìm thấy

- Thêm GitHub topic **`autoxide-module`** cho repo (Settings → About → Topics). Danh sách repo có topic đó là danh mục luôn
  mới: <https://github.com/topics/autoxide-module>.
- Thêm từ khoá crates.io `autoxide` và `autoxide-module` (`keywords` trong `Cargo.toml`; crates.io cho năm từ).
- Đặt `autoxide-module.toml` (bên dưới) ở gốc repo, và một dòng trong README: chạy với phiên bản Autoxide nào.

## File manifest của module (`autoxide-module.toml`)

Một file đề xuất để mô tả module cho người và công cụ. Nó là **siêu dữ liệu khai báo**: mã Rust vẫn là nguồn đúng, và một lệnh
kiểm trong CLI (`autoxide check`, dự kiến ở bước 71 G6) sẽ so file với mã. Trước đó file do người và danh sách cộng đồng đọc.

```toml
# autoxide-module.toml — ở gốc repo của module
manifest_version = 1

[module]
name = "bookmarks"                      # tên module; crate là autoxide-<tên>
crate = "autoxide-bookmarks"
summary = "Save and tag links."        # một dòng, tiếng Anh
license = "MIT OR Apache-2.0"
repository = "https://github.com/example/autoxide-bookmarks"
tier = "community"                      # official | community  (stable/experimental là hạng hợp đồng của core)

[compatibility]
autoxide = ">=0.19, <0.21"              # các bản core bản phát hành này đã được thử (khoảng SemVer)
rust-version = "1.99"                   # phiên bản Rust tối thiểu
databases = ["sqlite", "postgres"]      # cần cả hai để vào danh sách bên dưới

[provides]
permissions = ["bookmarks.read", "bookmarks.write"]
tables_prefix = "bookmarks_"
mcp_tools = ["bookmarks.list", "bookmarks.create"]
events = ["bookmarks.created"]

[requires]
# trait của module khác mà module này cần (context.require::<dyn …>())
modules = []
```

Luật: `databases` phải liệt kê cả `sqlite` và `postgres` mới được nhận vào danh sách cộng đồng (luật số một của core);
`autoxide` là khoảng các phiên bản đã phát hành mà bạn đã thử, không phải "bất kỳ".

## Tương thích phiên bản

Module phụ thuộc một bản Autoxide đã phát hành (`autoxide-kernel`, `autoxide-testkit`, `autoxide` ghim theo tag hay phiên bản).

| Core Autoxide | Hợp đồng module            | Tác giả module nên làm                                                    |
| ------------- | -------------------------- | ------------------------------------------------------------------------- |
| `0.x`         | Có thể đổi ở bản minor (có `#[deprecated]` trước) | Ghim đúng một minor (`=0.19`) và thử ở mỗi minor mới |
| `1.x`         | Ổn định: chỉ phá vỡ ở bản major | Phụ thuộc `1` (caret); khai `>=1.0, <2.0`                          |
| `2.0`         | Bản major mới              | Chuyển đổi, phát hành bản major mới của module                           |

Bản phát hành module chạy với nhiều bản core thì ghi khoảng đó ở `[compatibility]` và ở CI (thử với phiên bản thấp nhất và
mới nhất trong khoảng). Chỉ nâng khoảng sau khi bộ test qua trên phiên bản mới.

## Đưa module vào danh sách

**Danh sách cộng đồng** không có người gác cổng ngoài các điều kiện này, để ai cũng đăng được:

1. Module ở repo công khai theo giấy phép mã nguồn mở được OSI chấp nhận.
2. `cargo xtask check-module <thư mục>` từ bản checkout Autoxide qua được với nó (các luật module của core: sở hữu bảng,
   mã lỗi, bản dịch, tài liệu OpenAPI).
3. Test của nó qua trên **cả SQLite và PostgreSQL**, dùng `autoxide-testkit`.
4. Có README tiếng Anh (bản tiếng Việt càng tốt), `SECURITY.md` hay liên kết cách báo vấn đề, và `autoxide-module.toml`.
5. Repo có topic `autoxide-module`.

Để vào danh sách chọn lọc dưới đây, mở pull request thêm một dòng (theo thứ tự chữ cái của tên) vào file này. Người duy trì
kiểm năm điểm trên, không kiểm chất lượng mã, và có thể xoá dòng không còn đạt. Danh sách không phải sự bảo chứng.

### Danh sách cộng đồng

| Module | Tóm tắt | Phiên bản Autoxide | Repo |
| ------ | ------- | ------------------ | ---- |
| _chưa có_ | Ví dụ tham chiếu là [`examples/external-module`](../examples/external-module/README.vi.md) | `0.19` | trong repo này |

## Module chính thức và module cộng đồng

Module chính thức nằm ở `modules/` của repo này, do người duy trì Autoxide bảo trì, qua đủ gate và gắn nhãn hạng hợp đồng
(ổn định hay thử nghiệm). Module cộng đồng do tác giả của chúng bảo trì và không mang cam kết nào của dự án Autoxide. Module
cộng đồng được dùng rộng rãi có thể được đề xuất nhận thành module chính thức bằng một [RFC](rfcs/README.md).
