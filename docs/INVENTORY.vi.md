# Kho (`inventory`)

[English](INVENTORY.md)

Module `inventory` giữ tồn của mọi biến thể sản phẩm ở mọi kho hay cửa hàng của một khách. Mọi thay đổi vào một sổ
biến động chỉ thêm, và không bao giờ bán quá tồn, kể cả khi nhiều request đến cùng lúc, trên SQLite lẫn PostgreSQL.
Module khác dùng qua `autoxide_inventory::api`; API HTTP nằm dưới `/api/v1/inventory` (xem `docs/openapi.json`), tool
MCP tên `inventory.*`.

## Mô hình tồn

- **Kho** là nút tổ chức loại `warehouse` hay `store` (bước 34), hoặc nút gốc của khách (cửa hàng đơn lẻ giữ tồn ở
  đó). Nút loại khác trả `inventory.not_a_warehouse`.
- **Dòng tồn** là một biến thể ở một kho, giữ hai số:
  - `on_hand`: hàng đang nằm trong kho — luôn bằng tổng sổ biến động của dòng, không bao giờ dưới 0;
  - `available`: hàng còn bán được — `on_hand` trừ hàng đang giữ cho đơn. Có thể dưới 0 khi hàng ra khỏi kho mà
    tồn đã giữ hết (vd hàng hỏng) hay khi sản phẩm cho bán khi hết hàng.
  `reserved` (`on_hand − available`) được tính, không lưu.
- **Sổ biến động** (`GET /movements`) chỉ thêm: `receipt`, `sale`, `return`, `adjustment`, `count`, `transfer_out`,
  `transfer_in`, `expiry`, `supplier_return`. Sai thì sửa bằng một lần điều chỉnh mới.
- Số tồn chỉ đổi bằng một câu `UPDATE` có điều kiện (`rows::adjust`), không bao giờ đọc rồi ghi.

## Điều chỉnh tồn

`POST /adjustments` (`inventory.adjust`) đổi tồn ở một kho:

```json
{
  "org_id": "…kho…",
  "reference": "ADJ-2026-0042",
  "lines": [
    {"variant_id": "…", "quantity": -2, "reason_code": "damaged", "note": "Rơi vỡ"},
    {"variant_id": "…", "quantity": 50, "reason_code": "found",
     "unit_cost": {"amount": "120000", "currency": "VND"}}
  ]
}
```

- Mỗi dòng cần một lý do trong danh sách lý do `inventory.adjustment` của khách
  (`GET /api/v1/kernel/reasons/inventory.adjustment`); mã có sẵn: `damaged`, `lost`, `found`, `data_entry_fix`,
  `sample_use`, `expired`, `count_variance`. Khách đổi nhãn, thứ tự, tắt và thêm mã.
- `reference` giúp gọi lại an toàn: cùng mã và cùng nội dung trả lại lần điều chỉnh đã làm (`200`); cùng mã, khác
  nội dung là `reference_reused`.
- `on_hand` không bao giờ dưới 0 (`inventory.insufficient_stock`).
- Dòng dương có thể kèm `unit_cost` (vd tồn đầu kỳ); giá này cập nhật giá vốn bình quân.

## Giá vốn bình quân

Mỗi dòng tồn giữ giá vốn đơn vị bình quân gia quyền cho báo cáo lãi gộp: khi hàng có giá nhập vào,
`(tồn × giá cũ + số nhập × giá nhập) / (tồn + số nhập)`, làm tròn nửa lên xa số 0 về đơn vị nhỏ nhất của tiền tệ.
Không còn tồn thì giá vốn bằng giá nhập. Hàng vào không có giá (trả về, tìm thấy) giữ nguyên giá vốn. Giá vốn đổi thì
phát `inventory.cost_changed` (`catalog` dùng cho sàn giá).

## Điểm đặt hàng lại và cảnh báo

`PATCH /levels/{id}` (`inventory.settings.manage`) đặt `reorder_point`. `inventory.low_stock` phát một lần khi
`available` đi từ trên điểm đặt hàng lại xuống bằng hay dưới nó; `inventory.out_of_stock` khi đi từ trên 0 xuống 0
hay dưới. `GET /levels?below_reorder=true` và tool MCP `inventory.list_low_stock` liệt kê các dòng đó.

## Giữ chỗ (cho đơn hàng)

Module `orders` giữ hàng qua `autoxide_inventory::api::Inventory`:

1. `reserve` — giữ mọi dòng của đơn hoặc không gì, tới `expires_at` (tối đa `AUTOXIDE_INVENTORY_MAX_HOLD_HOURS`). Gọi
   lại cùng `order_id` và cùng nội dung trả lại chỗ giữ cũ; khác nội dung là `reference_reused`. Combo trừ từng thành
   phần; hàng số và dịch vụ được bỏ qua. Biến thể bán `backorder` hay `preorder` được giữ làm `available` âm
   (`backordered`).
2. `confirm` — đơn đã đặt: chỗ giữ không còn hạn.
3. `fulfil` — xuất một phần hay hết số đã giữ (`fulfilment_key` để gọi lại an toàn); ghi dòng sổ `sale`.
4. `release` — trả phần chưa xuất (huỷ đơn, thanh toán lỗi…). Trả hai lần chỉ cộng lại một lần.
5. `restock` — nhập lại hàng trả về (`reference` như `return:<id>`); dòng không bán lại được chỉ được ghi nhận.

Chỗ giữ chưa xác nhận quá `expires_at` được job `inventory.expire_reservations` (mỗi phút) trả lại. Mỗi phần giữ trừ
tồn bằng một câu `UPDATE` có điều kiện, theo thứ tự cố định, nên 100 người cùng mua cái cuối chỉ đúng một người được.

Quản trị xem chỗ giữ bằng `GET /reservations` (`inventory.reservations.read`) và gỡ chỗ bị kẹt bằng
`POST /reservations/{id}/release` (`inventory.adjust`; ghi lại người gỡ).

Module `inventory` cũng nghe `orders.order_confirmed`, `orders.order_cancelled` và `orders.return_received` (khi
đã cài module `orders`) và chạy đúng các hàm đó, nên event đến trước hay sau lời gọi trực tiếp, hay đến hai lần, chỉ
đổi tồn một lần.

## Chọn kho

`PUT /sources/{cửa hàng}` khai các kho phục vụ một cửa hàng hay kênh, theo `priority`; không khai thì cửa hàng bán tồn
của chính nó. `PUT /settings/{cửa hàng}`:

- `prefer_nearby` — kho cùng tỉnh với người mua trước (tỉnh lấy từ hồ sơ công khai của nút ở module `organizations`);
- `allow_split` — chia đơn cho nhiều kho khi không kho nào đủ mọi dòng (mặc định tắt: một đơn một kho).

Giữ chỗ thử trước một kho đủ mọi dòng; không có kho ứng viên nào thì trả `inventory.no_source`, không đủ hàng thì
`inventory.insufficient_stock` nêu từng dòng.

## Kiểm số liệu

`autoxide task inventory.verify_levels [tenant=<slug>] [org_id=<nút>]` so mọi dòng tồn với sổ và chỗ giữ của nó,
chỉ báo, không sửa. `autoxide doctor` chạy cùng phép kiểm trên mẫu 1.000 dòng mỗi khách.

## Permission

| Permission | Cho phép |
|---|---|
| `inventory.read` | Xem dòng tồn, sổ, lô, giá trị tồn |
| `inventory.adjust` | Điều chỉnh, áp dụng kiểm kê, gỡ chỗ giữ bị kẹt |
| `inventory.receive` | Phiếu nhập |
| `inventory.transfer` | Gửi và nhận phiếu chuyển kho |
| `inventory.count` | Kiểm kê |
| `inventory.reservations.read` | Chỗ giữ hàng của đơn |
| `inventory.settings.manage` | Điểm đặt hàng lại, kho phục vụ, cài đặt cửa hàng |
| `inventory.purchasing.read` / `.manage` | Nhà cung cấp, đơn đặt hàng, trả hàng nhà cung cấp |

Mọi permission theo nút tổ chức: người có grant `unit` ở kho A chỉ thấy và đổi kho A.

## Gói

Hạn mức `inventory.warehouses` (số nút giữ tồn); tính năng `inventory.multi_warehouse`, `inventory.transfers`,
`inventory.lots`, `inventory.serials`, `inventory.purchasing`.

## Cấu hình

| Biến | Mặc định | Nghĩa |
|---|---|---|
| `AUTOXIDE_INVENTORY_MAX_HOLD_HOURS` | `72` | Thời gian giữ hàng dài nhất của đơn chưa trả tiền |
| `AUTOXIDE_INVENTORY_EXPIRE_BATCH` | `200` | Số chỗ giữ quá hạn trả mỗi lần chạy job |
| `AUTOXIDE_INVENTORY_MAX_LINES_PER_RESERVATION` | `200` | Số dòng mỗi chỗ giữ |
| `AUTOXIDE_INVENTORY_PUBLIC_LOW_THRESHOLD` | `5` | Dưới số này tình trạng công khai là `low` |
