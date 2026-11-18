## D-20261118-step39-long-jobs · Lõi backup dời vào kernel; job chạy lâu được gia hạn giữ — 2026-11-18 — agent quyết

Chủ đề: Database, dữ liệu, job, event

Quyết định: Bước 39 mục A1, hai phần nền cho backup theo lịch.

1. **Lõi backup ở kernel** (`autoxide_kernel::backup`): `write_backup(database, migrator, modules, files, sink,
   scratch)` dựng file SQLite trong thư mục tạm riêng tư (`0700`, tự dọn kể cả khi lỗi), chép bảng qua `copy_modules`
   (đọc nguồn trong snapshot, không giữ transaction ghi nào trên nguồn), rồi đẩy `database.sqlite`, `files/<khoá>` và
   `backup.json` vào một `BackupSink`: `DirectorySink` (lệnh `autoxide backup`) hay `StorageSink` (nơi lưu file dưới
   một tiền tố, cho job). Định dạng bản backup giữ nguyên (`format` 1, D-073). File dưới `kernel/backups/` không bao giờ
   được chép — không thì bản sau chứa bản trước. `autoxide restore` vẫn ở `crates/app` (cần khởi động cả chương trình)
   nhưng dùng chung hằng số và hàm mở SQLite của kernel.
2. **Job chạy lâu:** `JobRegistration::timeout(limit)` cho một loại job chạy quá mặc định (4 phút 30, dưới hạn giữ 5
   phút). Khi `limit` dài hơn mặc định, worker chạy song song (`tokio::select!`) một vòng gia hạn: mỗi một phần ba hạn
   giữ, `UPDATE <module>_jobs SET claimed_until = now + hạn giữ WHERE id = ? AND claimed_by = <worker>`; 0 dòng (job bị
   trả về hàng đợi, worker khác lấy) thì dừng job và không ghi kết quả. Một lần lỗi database khi gia hạn chỉ ghi log —
   hạn giữ còn hai phần ba. `Worker::lease` (ẩn khỏi tài liệu) đặt hạn giữ ngắn cho test.

Lý do: backup trên dữ liệu cỡ thật chạy lâu hơn 4 phút 30 — không gia hạn thì job bị cắt, hay worker khác lấy lại và
hai bản backup chạy chồng (đặc tả bước 39, "Chỗ dễ sai").

Đã cân nhắc: tăng hạn giữ chung cho mọi job (job chết giữa chừng phải chờ lâu hơn mới được chạy lại); chạy backup ngoài
job queue (mất lịch, chạy lại và theo dõi của job queue).
