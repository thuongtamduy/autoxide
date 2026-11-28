## D-20261128-kyc-article-18 · Hồ sơ định danh ghi mục NĐ 248 Điều 18; cổng chỉ chặn việc bán — 2026-11-28 — agent quyết

Chủ đề: Pháp luật và tuân thủ

Quyết định: Hồ sơ KYC của `compliance` thêm năm mục (m0024): `birth_date`, `address`, `representative_name`,
`document_country`, `document_expires_on`. Nhà cung cấp trả qua `VerificationOutcome` (kernel, thêm trường và hàm
dựng — struct `#[non_exhaustive]` nên không phá vỡ); người duyệt ghi qua `PUT /api/v1/compliance/kyc/profiles/{id}/details`
(`compliance.kyc.review`, người đăng nhập). Mục nào bắt buộc là dữ liệu có ngày hiệu lực: quy tắc
`kyc.required_activities` phiên bản 2 từ 01/01/2027 (`seller_details` theo loại bên, `passport_min_valid_months: 6`).
Thiếu mục thì cổng chỉ chặn **bán** (`compliance.kyc_required`, `profile_status = incomplete`, mục thiếu ở `missing`);
trạng thái `verified` không đổi. Hồ sơ không còn hiệu lực sau hạn giấy tờ (`expires_at` rút theo
`document_expires_on`). Thân quy tắc `kyc.required_activities` không phải JSON là lỗi, không coi là "không bắt buộc".

Căn cứ: NĐ 248/2026/NĐ-CP Điều 18 (cá nhân: họ tên, ngày sinh, số định danh; tổ chức, hộ kinh doanh: tên, trụ sở,
số định danh, người đại diện; cá nhân nước ngoài: hộ chiếu còn hạn ít nhất 6 tháng), áp dụng 01/01/2027 (Điều 52) theo
các bản tổng quan — cần đối chiếu văn bản gốc / người có chuyên môn pháp lý.

Lý do: core giữ mục luật đòi sàn phải xác thực (không giao hết cho nhà cung cấp eKYC: sàn phải trình được khi cơ quan
hỏi). Chặn ở việc bán chứ không hạ trạng thái định danh: KYC còn dùng cho đăng ký gói, livestream, tiếp thị liên kết —
Điều 18 nói về người bán. Hồ sơ đã xác thực trước 2027 thiếu mục bổ sung được mà không phải định danh lại. Hạn hộ chiếu
tính từ ngày xác thực (`verified_at`) như nghị định nói "lúc xét duyệt".

Đã cân nhắc:
- Đưa hồ sơ thiếu mục về `pending` khi xác thực: chặn cả việc không phải bán, và hồ sơ cũ thì không về được.
- Bắt mục khi duyệt tay (`approve`): thêm đầu vào vào một lệnh đang có; một endpoint ghi mục riêng dùng được cả cho hồ
  sơ đã xác thực.
- Lưu mục ở nhà cung cấp, core chỉ giữ mã tham chiếu: core không kiểm được điều kiện bán.

Còn mở: livestream và tiếp thị liên kết chưa có module hỏi `KycGate`.
