## D-20261128-time-duration-ops · `time::Duration`: toán tử với `Timestamp` kẹp biên, `checked_*` trả `None`, đường cũ qua `Into<Duration>` — 2026-11-28 — agent quyết

Chủ đề: Kiến trúc module và kernel

Quyết định: cài mục 3 của D-20261128-library-layer-plan như sau.

1. `Duration` bọc một `i64` micro giây (không bọc kiểu `jiff`). Hàm dựng `from_*` là `const fn`, bão hoà ở `MAX`/`MIN`
   (≈ ±292.000 năm) thay vì tràn; `as_*` cắt về 0. Không `serde`.
2. "Cộng trừ với `Timestamp`, tràn thì lỗi, không panic": `Timestamp::checked_add`/`checked_sub` trả `None` khi ra ngoài
   năm ±9999; toán tử `+`/`-` không bao giờ dừng chương trình mà **kẹp** về `Timestamp::MIN`/`MAX`. Lý do: module dùng
   toán tử để tính mốc giữ dữ liệu từ cấu hình (`now - Duration::from_days(ngày)`); bắt `?` ở mỗi chỗ làm code dài mà mốc
   kẹp vẫn so sánh đúng chiều. Khoảng do người dùng nhập thì dùng `checked_*`.
3. Đường cũ (D-20261127-transition-paths): `impl From<jiff::SignedDuration> for Duration`, và hàm kernel nhận khoảng thời
   gian lấy `impl Into<Duration>` (`Timestamp::checked_add/sub`, `OneTimeTokens::issue`), nên chỗ gọi cũ truyền
   `SignedDuration` vẫn biên dịch. Hàm nhận múi giờ (`date_in`, `to_rfc3339_in`, `in_zone`, `Date::start_of_day_in`)
   lấy `&impl time::Zone` — trait niêm phong, kernel cài cho `TimeZone` và `jiff::tz::TimeZone`. Kiểu mới có tên mới
   khi hàm cũ trả kiểu `jiff`: `Timestamp::since` (cũ `duration_since`/toán tử `Timestamp - Timestamp`),
   `Timestamp::to_local` (cũ `in_zone`), `Date::day_of_week` (cũ `weekday`). Khi module cuối chuyển: gắn
   `#[deprecated]`, rồi gỡ ở bản sau; toán tử `Timestamp - Timestamp` đổi sang trả `Duration` ở bản gỡ đó.
4. Trường công khai kiểu `SignedDuration` (`StepUp::window`) và `Timestamp::truncate(jiff::Unit)` chuyển cùng cụm module
   dùng chúng (`auth`, cụm 3), không chuyển ở PR này.

Lý do: cụm 2 (`media`, `comments`, `forms`) chỉ cần `Duration`; các kiểu lịch làm cùng PR để cụm 3 không phải chờ. Đường
cũ qua `Into`/trait niêm phong giữ cho 300+ chỗ gọi `SignedDuration` ở module chưa chuyển biên dịch, không phải sửa
hàng loạt ngoài cụm.

Đã cân nhắc: toán tử trả `Result<Timestamp, Error>` (mọi phép trừ mốc thành `(now - d)?`, test thêm `unwrap`); toán tử
panic như trước (trái mục 3); đổi ngay kiểu trả của `duration_since`/`Timestamp - Timestamp` (vỡ mọi chỗ gọi ngoài cụm).
