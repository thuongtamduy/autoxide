## D-20261122-block-registry · Sổ khối của kernel: khai qua `Module::blocks`, schema props là phần con tự kiểm, danh sách cho mọi người đăng nhập — 2026-11-22 — agent quyết

Chủ đề: Kiến trúc module và kernel

Quyết định (bước 67 mục 1, chi tiết hoá D-229):

1. Module khai khối bằng `Module::blocks() -> Vec<BlockType>` (mặc định rỗng — D-038). Tên luôn là
   `<module>.<khối>`, kể cả khối nền của `content` (`content.section`, không phải `section` trần như bảng phác
   thảo ở `docs/steps/67-page-builder.md`): một luật đặt tên, không có ngoại lệ, và tên module ngay trong tên khối.
   Nhãn và gợi ý cho AI là câu tiếng Anh trong `BlockType::new`, dịch qua `text:<câu>` như mô tả permission —
   `cargo xtask check` chặn câu chưa dịch (không có cặp `label_key` / `ai_hint_key` riêng).
2. Khối cần dữ liệu khai `with_data()` và gắn `BlockData` lúc `start` bằng `StartContext::block_data`; khai mà
   không gắn (hay gắn cho khối không khai) thì chương trình không khởi động. `Blocks::resolve` gom theo loại, gọi
   mỗi loại một lần song song, hạn 2 giây; lỗi, hết giờ, số phần tử sai hay `None` → `null` cho khối đó.
3. Schema props là **phần con của JSON Schema 2020-12 do kernel tự kiểm** (`autoxide_kernel::blocks::schema`, ~300
   dòng): `type`, `properties`, `required`, `additionalProperties`, `items`, `enum`, `const`, độ dài, khoảng,
   số phần tử, `format` `uuid` / `date-time`, cùng `x-responsive` (`{base, md?, lg?}`), `x-media` (ID `media`),
   `x-link` (luật link của D-102, thêm `tel:`). Từ khoá ngoài danh sách là lỗi khai báo lúc khởi động; object nào
   cũng phải `additionalProperties: false`; chuỗi có dạng thẻ HTML bị từ chối. Mọi lỗi props trả `FieldError` có
   đường dẫn, để `content` gom báo một lần (D-083).
4. `GET /api/v1/kernel/blocks` cho **mọi người đã đăng nhập** (không đòi `content.pages.write` như bảng API của đặc
   tả): kernel không được biết permission của `content`, và danh sách chỉ là siêu dữ liệu như tài liệu OpenAPI vốn
   công khai. Lọc theo module đang chạy (`AUTOXIDE_MODULES` — module tắt không vào sổ), module trong gói của khách
   và tính năng của khối (phải do chính module khai).

Lý do: khối do module cung cấp nên sổ phải ở kernel và có trước khi module thương mại khai khối; tự kiểm phần con
giữ "từ chối mặc định" đúng nghĩa (schema chỉ nói điều được kiểm thật), không thêm cây phụ thuộc lớn cho một việc
vài trăm dòng (như D-045, D-057), và cùng chỗ với bộ kiểm schema của AI (`ai::schema`).

Đã cân nhắc: thư viện `jsonschema` (kéo `fancy-regex`, `reqwest` tuỳ chọn và nhiều crate; nhận cả từ khoá mà giao
diện và mô hình AI không hiểu như `$ref`, `oneOf`, `pattern` — schema khối sẽ khó đọc cho người làm giao diện);
permission riêng `kernel.blocks.read` (phải cấp thêm cho mọi role biên tập mà không bảo vệ được gì); tên khối nền
không tiền tố (hai luật đặt tên, dễ trùng giữa module).
