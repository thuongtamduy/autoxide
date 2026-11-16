# Đơn vị hành chính và địa chỉ

[English](ADDRESS.md)

Module `address` giữ danh mục đơn vị hành chính chính thức, dùng chung mọi khách hàng — với Việt Nam
từ 01/7/2025: tỉnh và thành phố trực thuộc trung ương (cấp 1) và xã, phường, đặc khu (cấp 2), không có
cấp huyện — và kiểm địa chỉ cho các module khác (hồ sơ công khai của cửa hàng, sau này là địa chỉ giao
hàng của người mua).

## Đọc

Không cần đăng nhập; kết quả có `ETag` đổi khi nhập danh mục mới và
`Cache-Control: public, max-age=86400`.

| Request | Kết quả |
|---|---|
| `GET /api/v1/address/provinces?country=VN&as_of=YYYY-MM-DD` | tỉnh còn hiệu lực ngày đó (mặc định: hôm nay), theo mã |
| `GET /api/v1/address/provinces/{code}/communes` | xã của một tỉnh |
| `GET /api/v1/address/units?q=ha noi&province=01&level=2` | tìm theo tên, có dấu hay không dấu |
| `GET /api/v1/address/units/{code}` | một đơn vị và đơn vị cấp trên |
| `POST /api/v1/address/validate` | `{country, province_code, commune_code, line}` → địa chỉ kèm tên, hay 422 |

Tool MCP: `address.list_provinces`, `address.list_communes`, `address.search_units`,
`address.validate_address`.

## Nhập danh mục

Máy chủ kèm sẵn danh mục Việt Nam hiệu lực từ 01/7/2025 (34 tỉnh / thành, 3.321 xã, phường, đặc khu theo
QĐ 19/2025/QĐ-TTg; lấy từ provinces.open-api.vn v2, nơi đăng lại danh mục của cơ quan thống kê — cần chắc
về pháp lý thì đối chiếu văn bản gốc). Nhập một lần cho mỗi bản cài:

```bash
autoxide task address.import
```

`autoxide doctor` cảnh báo khi chưa nhập, và khi bản cập nhật kèm danh mục mới hơn.

Để nhập danh mục khác (khi nhà nước đổi), lấy từ nguồn chính thức — **QĐ 19/2025/QĐ-TTg** (bảng danh mục và mã
số đơn vị hành chính, hiệu lực từ 01/7/2025) và danh mục điện tử của cơ quan thống kê ("Danh mục hành
chính Việt Nam") — không lấy bản chép không chính thức, không sinh dữ liệu. Đặt hai file vào một thư mục:

`manifest.json`:

```json
{
  "country": "VN",
  "title": "QĐ 19/2025/QĐ-TTg",
  "source_url": "<địa chỉ file chính thức đã tải>",
  "retrieved_on": "YYYY-MM-DD",
  "effective_from": "2025-07-01"
}
```

`units.csv` (UTF-8, dấu phẩy, dòng đầu đúng như dưới; mã giữ số 0 đầu):

```text
level,code,name,kind,parent_code
1,01,<tên tỉnh>,city,
2,00004,<tên xã>,ward,01
```

`kind` là `province`, `city`, `commune`, `ward` hay `special_zone`; `parent_code` của xã là mã tỉnh của
nó. Rồi chạy:

```bash
autoxide task address.import path=/duong/dan/toi/thu/muc
```

Nhập lại cùng file không đổi gì. Khi nhà nước đổi danh mục, nhập bộ mới: đơn vị biến mất hay đổi tên,
loại, tỉnh thì đóng từ ngày mới và mở đơn vị mới — địa chỉ ghi trước đó giữ tên lúc ghi.
`autoxide doctor` cho biết danh mục đang dùng.
