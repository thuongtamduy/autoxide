# Thuế

[English](TAX.md)

Module `tax` tính thuế từ **dữ liệu**: nhóm thuế và thuế suất có ngày hiệu lực, tỉ lệ khấu trừ thay
của sàn, và các quy tắc (vd khi nào phải dùng hoá đơn máy tính tiền) nằm trong gói dữ liệu thuế,
không nằm trong code. Máy chủ có sẵn gói cho Việt Nam. Module khác (bán hàng, hoá đơn, sổ cái) tính
thuế qua `autoxide_tax::api::TaxEngine`.

> Mọi thuế suất trong gói đi kèm đều ghi căn cứ pháp lý và chú thích "cần đối chiếu văn bản gốc".
> Hãy nhờ người có chuyên môn kiểm trước khi dùng thật.

## Gói Việt Nam

| Nhóm | Thuế suất | Ghi chú |
|---|---|---|
| `vat.standard` | 10% | Luật Thuế GTGT 48/2024/QH15 |
| `vat.standard_reducible` | 8% tới hết 31/12/2026, sau đó 10% | mức giảm 2%; hàng không được giảm dùng `vat.standard` |
| `vat.reduced_5` | 5% | |
| `vat.zero` | 0% | xuất khẩu |
| `vat.exempt` | không chịu thuế | ghi "không chịu thuế GTGT", không phải 0% |
| `excise.beer` | thuế TTĐB 65%, rồi GTGT 10% trên giá đã có TTĐB | mẫu |

Sàn có chức năng thanh toán khấu trừ thay cho hộ kinh doanh (NĐ 117/2025/NĐ-CP): hàng hoá 1% GTGT +
0,5% TNCN, dịch vụ 5% + 2%, sản xuất – vận tải 3% + 1,5%. Doanh nghiệp tự kê khai: không khấu trừ. Core hiện làm:
tính số tiền (`POST /api/v1/tax/withholding`) và ghi sổ khi một lần quyết toán yêu cầu (`ledger`, bút toán
`TaxWithheld`). Core **chưa** làm (cùng module sàn, sau 1.0): tự khấu trừ ở mỗi lần chi trả cho người bán là hộ kinh
doanh, lập chứng từ khấu trừ (loại chứng từ `withholding_certificate` có nhưng chưa ai tạo), và chuẩn bị dữ liệu tờ
khai thay — tới lúc đó sàn tự làm.

Gói tự được nhập trong vòng một giờ sau khi máy chủ chạy (và mỗi khi bản cập nhật có gói mới hơn).
Muốn nhập ngay, hay nhập gói khác:

```bash
autoxide task tax.import_pack
autoxide task tax.import_pack path=/duong/dan/goi.json
```

hay `POST /api/v1/tax/packs` (nhân sự nền tảng có `tax.packs_manage`). Chỉ nhận `version` lớn hơn bản
đã nhập; nhập lại cùng gói không đổi gì. `autoxide doctor` cảnh báo khi một thuế suất hết hạn trong
`AUTOXIDE_TAX_EXPIRY_WARNING_DAYS` (60) ngày mà không có thuế suất nối tiếp — vd mức 8% được gia hạn qua
2026 thì nhập gói ghi điều đó.

## Tính thuế

```bash
curl -X POST https://api.example.com/api/v1/tax/quote \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"jurisdiction": "VN", "tax_point": "2026-11-17", "price_mode": "inclusive", "currency": "VND",
       "lines": [{"line_id": "1", "category": "vat.standard_reducible", "amount": {"amount": "108000", "currency": "VND"}}]}'
```

- `price_mode`: `exclusive` (giá chưa thuế) hay `inclusive` (tách thuế ra).
- Thuế làm tròn **một lần cho mỗi thuế suất trên cả chứng từ**, rồi chia lại về dòng: tổng các dòng
  luôn khớp tổng. Tầng sau (GTGT) tính trên giá cộng các tầng trước (TTĐB).
- `tax_point` là một ngày theo lịch, không phải một thời điểm. Đơn hàng và hoá đơn tính nó bằng
  `TaxEngine::tax_date`: ngày theo múi giờ của vùng thuế người bán (quy tắc `tax_date.time_zone` của gói;
  Việt Nam: `Asia/Ho_Chi_Minh`), không có thì theo `organizations.timezone` của nút, không có nữa thì UTC.
  Vì vậy đơn đặt lúc 00:30 ngày 01/01/2027 giờ Việt Nam (17:30 UTC ngày 31/12) tính thuế suất của ngày
  01/01. Không có thuế suất ngày đó: `422 tax.rate_missing` — engine không bao giờ đoán.
- **Thuế của đơn là ước tính; hoá đơn quyết định.** Luật gắn thời điểm xác định thuế GTGT với lúc chuyển
  giao quyền sở hữu hàng (giao hàng) hay hoàn thành dịch vụ. Đơn tính thuế theo ngày đặt; hoá đơn của đơn
  tính theo ngày lập của nó (`orders.invoice_timing`, mặc định `on_delivered`). Thuế suất đổi ở giữa thì hoá
  đơn dùng thuế suất mới và chứng từ của đơn (`GET …/orders/me/orders/{id}/documents`) hiện
  `differs_from_order: true` — xử lý chênh lệch với người mua. Cờ so thuế từng dòng và tổng tiền, bỏ qua lệch làm
  tròn 1 đơn vị tiền nhỏ nhất mỗi dòng (2 với dòng huỷ một phần, hoá đơn làm tròn lại): đơn giá chưa thuế ghi phí giao (đã gồm thuế) lên hoá đơn bằng phần trước thuế,
  nên tổng hoá đơn có thể lệch đơn 1 đồng. Đơn trộn giá gồm thuế và giá chưa thuế luôn được báo — một hoá đơn chỉ có
  một chế độ giá nên không ghi đúng được. Cờ là `true` thì sửa hoá đơn bằng hoá đơn điều chỉnh
  (`POST /api/v1/invoices/documents/{id}/adjust`) hay hoá đơn thay thế (`POST /api/v1/invoices/documents/{id}/replace`). Nháp hoá đơn tính vào ngày khác được tính lại khi lập. Hoá đơn điều chỉnh, thay thế giữ ngày thuế
  (thuế suất) của hoá đơn gốc. Bút toán sổ cái của đơn và hoá đơn ghi theo cùng ngày thuế. `autoxide doctor`
  cảnh báo khi vùng thuế đã nhập không khai `tax_date.time_zone`.
- Câu trả lời có `snapshot` (thuế suất và ID, phiên bản gói, cách làm tròn): lưu kèm chứng từ.

`POST /api/v1/tax/withholding` tính số sàn khấu trừ khi trả tiền cho người bán. Tool MCP: `tax.quote`,
`tax.rates`.

## Hồ sơ thuế

Mỗi nút tổ chức có thể có hồ sơ thuế — mã số thuế (10 hay 13 số, `0101234567-001` cho đơn vị phụ thuộc,
hay số định danh cá nhân 12 số), chế độ thuế, ngành nghề, doanh thu ước tính năm, tên pháp lý, địa chỉ:
`PUT /api/v1/tax/profiles/{org_id}` (`tax.profiles_manage` ở nút). Nút chưa có hồ sơ dùng hồ sơ của nút
cấp trên gần nhất. Chế độ thuế quyết định loại hoá đơn: `vat_credit` → hoá đơn GTGT; `vat_direct`,
`household` → hoá đơn bán hàng, không tách GTGT trên dòng; `exempt` → không thuế. Hộ kinh doanh có
doanh thu từ 1 tỷ đ/năm phải dùng hoá đơn khởi tạo từ máy tính tiền (`TaxEngine::invoice_requirements`,
NĐ 70/2025/NĐ-CP — cần đối chiếu lộ trình mới nhất).

## Đọc dữ liệu

`GET /api/v1/tax/jurisdictions`, `/categories`, `/rates?on=YYYY-MM-DD`, `/withholding-rates`, `/rules`,
`/packs` (`tax.read`).
