## D-20261126-migrate-every-placement · Migrate tự động nâng mọi placement và làm tiếp qua placement hỏng — 2026-11-26 — agent quyết

Chủ đề: Database, dữ liệu, job, event

Quyết định:

- `AUTOXIDE_MIGRATE_ON_START=true` áp dụng migration của database chính **rồi mọi placement** (như
  `autoxide migrate up --all`), không chỉ database chính. Placement hỏng chỉ ghi log cảnh báo: máy chủ vẫn khởi
  động, khách trên placement đó nhận 503 `tenant_maintenance` như trước. Database chính hỏng thì vẫn dừng khởi động.
- `Placements::up_all` thử **mọi** placement: placement hỏng thành `behind`, các placement sau vẫn chạy; lỗi trả về
  nêu tên từng placement hỏng. `Placements::up_all_report` trả phần đã xong cùng danh sách hỏng (`MigrationReport`,
  `PlacementFailure`). `autoxide migrate up --all` in phần đã xong ra stdout, từng placement hỏng ra stderr, mã thoát
  1. Lượt migrate placement qua API (`POST /api/v1/kernel/placements/migrations`) cũng làm tiếp, kết thúc `failed`
  kèm lỗi của từng placement.
- `compose.yml` và `deploy/kubernetes/autoxide.yaml` chạy `migrate up --all`; tài liệu cập nhật hướng dẫn `--all`
  (ở chế độ một khách nó giống `migrate up`).

Lý do: bước 71 nhóm G3 (D-20261126-road-to-1-0) — nâng cấp phải là một bước. Trước đây khách có database riêng kẹt
`behind` sau mỗi lần nâng nếu người vận hành quên `--all`, và một khách hỏng chặn mọi khách đứng sau theo tên, dù lỗi
chỉ là của riêng khách đó. Mỗi database tự nhớ phần đã xong, nên làm tiếp không gây mất nhất quán; chạy lại vẫn bỏ qua
phần đã áp dụng.

Đã cân nhắc: để `serve` từ chối khởi động khi có placement hỏng — khách trên `main` và các placement tốt bị kéo sập
theo một khách; giữ dừng ở lỗi đầu tiên và để người vận hành sửa từng cái — chậm và các khách sau phải chờ vô cớ; đổi
kiểu trả về của `up_all` — phá vỡ chỗ gọi đang `unwrap()` (testkit, test của module) mà không được gì.

Thay: phần "`AUTOXIDE_MIGRATE_ON_START=true`: áp dụng migration còn chờ của database chính (placement của chế độ
`multi` vẫn cần `autoxide migrate up --all`)" của D-20261121-migrate-on-start
