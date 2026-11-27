## D-20261126-upgrade-rollback · `autoxide upgrade` và `autoxide rollback --to`: nâng và lùi bản thành một lệnh — 2026-11-26 — agent quyết

Chủ đề: Database, dữ liệu, job, event

Quyết định: nâng và lùi bản là lệnh của binary, chạy lại an toàn, giống nhau trên SQLite và Postgres, một khách hay nhiều
khách (bước 71 G3, D-20261126-road-to-1-0). Thiết kế đã qua kiểm độc lập (13 điểm, 26/11/2026); cài theo năm PR
(a) manifest + rollback, (b) backup đúng schema nguồn, (c) `upgrade` + báo cáo, (d) `doctor` báo bản mới, (e)
compose / Kubernetes / DEPLOY.

1. **Bản phát hành nào có migration nào** — `crates/app/migration-releases.json`, nhúng vào binary: mỗi migration
   (`<module>/<tên>`) một lần, kèm bản phát hành đầu tiên có nó, và danh sách bản. Migration đã phát hành không bao giờ
   bị bỏ, nên tập của bản X = mọi migration có bản đầu ≤ X (so theo SemVer trong Rust, không so chuỗi). Migration chưa có
   trong file là của bản đang phát triển. Lúc phát hành: `cargo xtask migration-releases record X.Y.Z` (đọc
   `autoxide migrate manifest`, lệnh in danh sách migration của binary, không cần database). Các bản đã phát hành điền
   bằng `scripts/migration-releases-backfill.sh`: chạy thật `migrate up` + `migrate status` của image Docker từng bản
   (`AUTOXIDE_MODULES=all`) — chính xác hơn đọc mã nguồn của tag (migration sinh trong kernel cho module, module thử
   nghiệm tắt mặc định). Có image từ 0.14.0; bản cũ hơn không có image nên không lùi về được. Không thêm cột phiên bản vào
   bảng lịch sử: không phải ALTER ~60 bảng × N placement, chạy được với database cũ, và nhãn phiên bản không sai giữa hai lần
   phát hành. Dựa vào TÊN migration, không đổi khi viết lại migration sang `Plan` (D-20261126-migration-rewrite-proof).
2. **`autoxide rollback --to X.Y.Z`** chạy bằng binary MỚI (chỉ nó có `down` của migration mới). Gỡ mọi migration đã áp
   mà không có trong bản X, mới nhất trước: module ngược thứ tự đăng ký (phụ thuộc), trong module ngược thứ tự chạy
   (`Migrator::rollback_plan` / `rollback`). Placement trước (theo tên), database chính sau cùng; placement nào hỏng thì
   không lùi database chính. Giữ khoá chính rồi khoá placement và chỗ `active`, ghi một lượt `kind = rollback` trong
   `kernel_migration_runs` (migration mới `m0043_migration_run_kind`: `kind`, `from_version`, `to_version`), đóng lượt
   TRƯỚC khi lùi database chính vì lùi xa có thể bỏ chính các cột đó. `--dry-run` in từng database: migration sẽ gỡ,
   bảng / cột sẽ mất kèm số dòng (chạy các `down` schema trong một transaction, so bảng / cột trước và sau, huỷ
   transaction; data upgrade chỉ liệt kê). Chạy thật có dữ liệu mất cần `--accept-data-loss`; luôn cần `--yes` (xác
   nhận đã dừng mọi máy chủ bản hiện tại — kernel chưa có sổ tiến trình để tự kiểm). Từ chối khi phải giữ một migration
   chạy sau một migration bị gỡ, hay gặp migration khai `MigrationTrait::irreversible() == true` (hàm mới, mặc định
   `false`, D-038; bước thu hẹp đánh dấu như vậy phải ghi ở mục changelog "cần làm khi nâng cấp") — khi đó khôi phục
   backup. Binary ghép module ngoài: module không có trong bảng bị coi là chưa phát hành, `--dry-run` cho thấy nó bị gỡ.
3. **Backup trước nâng cấp** (PR b): dựng bản nháp đúng tập migration nguồn đã áp (không chạy `up` của binary mới), manifest
   ghi tên migration đã áp, giữ dữ liệu module đang tắt; từng placement backup ngay trước migration đầu tiên của nó, chỉ
   khi có migration chờ, chạy lại dùng lại backup đó; `--backup=recent` nhận bản tốt trong `kernel_backups` mới hơn N giờ;
   mặc định ghi vào kho của backup định kỳ (`kernel/backups/<db>/upgrade-<from>-<to>/`), `--backup-dir` ghi đè (0700),
   `--no-backup` cho PITR. CI restore bằng binary bản trước, cả hai database.
4. **`autoxide upgrade`** (PR c): bảng tương thích trong binary (`OLDEST_UPGRADE_FROM`, `ManualStep {id, since, summary}`
   + `--confirm <id>`), backup như 3, migrate database chính rồi placement (`--parallel` mặc định 2, SQLite luôn 1), kiểm
   của `doctor` không gọi mạng, báo cáo JSON (không bao giờ có URL placement; lỗi qua redaction), lượt `kind = upgrade`.
   Chạy lại tự làm tiếp (không có `--resume`). Mã thoát 0 xong, 1 hỏng, 2 cấu hình sai, 3 xong một phần.
5. **`doctor` báo bản mới** (PR d): chỉ báo "có tag mới hơn"; chỉ HTTPS, không redirect sang host khác, trần 64 KB, 3 giây;
   bật mặc định, tắt bằng `AUTOXIDE_UPDATE_CHECK=false`; không tải bảng tương thích từ xa.

Lý do: G3 yêu cầu nâng một bước và lùi được. Bảng theo tên migration là cách duy nhất biết "bản X gồm migration nào" mà
chạy được với database đã cài từ trước, không đụng bảng lịch sử của từng module. Lùi bằng `down` giữ được dữ liệu ghi sau
khi nâng ở các bảng không đổi; backup là lưới an toàn khi `down` không đủ. Đo dữ liệu mất bằng transaction huỷ thay vì đọc
mã `down`: chính xác cho mọi kiểu `down`, và cả hai database chạy DDL trong transaction.

Đã cân nhắc: cột `release` trong bảng lịch sử (bản đầu, kiểm độc lập bác: nhãn sai khi build giữa hai lần phát hành, so
chuỗi sai, ALTER hàng loạt); đọc tên migration từ cây mã nguồn của tag (bỏ sót migration sinh trong kernel, module thử
nghiệm); bảng `kernel_upgrades` riêng (trùng việc với `kernel_migration_runs`, không ghi được lượt tạo ra chính nó); lùi
chỉ bằng khôi phục backup (mất dữ liệu ghi sau khi nâng); lùi bằng binary cũ (không có `down` của migration mới);
Atlas / Flyway (thêm một hệ migration thứ hai).
