## D-20261118-step39-scheduled-backups · Backup theo lịch vào nơi lưu file, giữ N bản, không chép file mặc định — 2026-11-18 — agent quyết

Chủ đề: Vận hành và hiệu năng

Quyết định: Bước 39 mục A1.

- Job `kernel.backup` của module `kernel`, lịch `kernel.backup` mỗi `AUTOXIDE_BACKUP_INTERVAL_HOURS` (mặc định 24, `0`
  tắt, 1–720), timeout riêng 6 giờ (gia hạn giữ, D-20261118-step39-long-jobs). Lịch đặt `on_control_only`: MỘT job mỗi
  chu kỳ, job duyệt mọi database máy chủ này lo (`DatabaseRegistry::databases`, gồm database riêng của placement) và
  backup từng cái — thay vì mỗi database một job như đặc tả phác: sổ `kernel_backups` nằm ở database chính, một job
  ghi sổ cho mọi database đơn giản hơn. Chỉ có lịch khi có nơi lưu file chung (module `files`).
- Mỗi database một bản dưới `kernel/backups/<database>/<id>/` của nơi lưu file chung, bằng đúng `write_backup` của
  `autoxide backup`. `AUTOXIDE_BACKUP_INCLUDE_FILES` mặc định `false`: bản backup nằm trên chính nơi lưu đó, chép file
  sang cùng chỗ không bảo vệ gì mà nhân đôi dung lượng — tài liệu hướng dẫn sao bucket / thư mục sang nơi khác.
  `AUTOXIDE_BACKUP_KEEP` mặc định 7: giữ 7 bản `succeeded` mới nhất mỗi database; xoá file và dòng của bản cũ hơn và
  của bản `failed` cũ hơn bản giữ lại cũ nhất (bản `failed` mới hơn để lại cho người vận hành đọc lỗi). Bản hỏng thì
  dọn ngay phần đã ghi.
- Bảng mặt điều khiển `kernel_backups` (migration `m0036_create_backups`): unique `active_key` (= tên database khi
  `running`, `NULL` khi xong) chặn hai lượt cùng lúc cho một database trên cả hai database; `heartbeat_at` cập nhật mỗi
  phút, lượt `running` không nhịp quá 15 phút (tiến trình chết) bị lượt sau đánh `failed`.
- Số đo: `backup_runs_total{outcome}`, `backup_duration_seconds{database}`, `backup_size_bytes{database}`,
  `backup_last_success_timestamp_seconds{database}`; cảnh báo `AutoxideBackupTooOld` (ngưỡng 48 giờ = 2 × mặc định,
  ghi chú trong file luật phải sửa khi đổi chu kỳ) và `AutoxideBackupFailed`. `autoxide doctor` thêm bước
  `kernel.backup_age`: cảnh báo khi tắt lịch, khi không có nơi lưu, hay khi một database không có bản tốt trong 2 ×
  chu kỳ.
- Lỗi của job chỉ nằm ở `kernel_backups.error`, log và số đo (không qua HTTP), nên không thêm mã lỗi
  `kernel.backup_running` / `kernel.backup_storage_missing` của đặc tả; job không trả lỗi khi một database hỏng (lượt
  sau theo lịch thử lại — lỗi kéo dài như đầy đĩa sẽ lặp lại mỗi lần thử ngay). Chưa phát event
  `kernel.backup_completed` / `_failed`: chưa module nào cần; thêm khi `notifications` muốn báo.
- `ScheduledBackups` được công bố (`context.provide`) để test và lệnh vận hành sau này gọi `run()` trực tiếp.

Lý do: đặc tả bước 39; Luật Bảo vệ dữ liệu cá nhân 2025 (91/2025/QH15, hiệu lực 01/01/2026): dữ liệu đã xoá theo yêu cầu
còn trong bản backup tới khi xoay vòng — DEPLOY ghi rõ để khách đưa vào thông báo xử lý dữ liệu; nơi đặt bucket theo vị
trí dữ liệu của khách (Luật An ninh mạng 116/2025/QH15, NĐ 53/2022/NĐ-CP) — cần đối chiếu văn bản gốc / người có chuyên
môn pháp lý.

Đã cân nhắc: mỗi database một job (sổ phải nằm ở database chính, job ở database placement không ghi sổ được); chép file
mặc định (nhân đôi dung lượng, không bảo vệ khi mất nơi lưu); endpoint HTTP chạy backup (lệnh nặng gọi được từ Internet —
job và CLI là đủ).
