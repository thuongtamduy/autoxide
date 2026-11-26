## D-20261126-migrate-every-placement · Migrate tự động nâng mọi placement và làm tiếp qua placement hỏng — 2026-11-26 — agent quyết

Chủ đề: Database, dữ liệu, job, event

Quyết định:

- `Placements::up_all` thử **mọi** placement: placement hỏng thành `behind`, các placement sau vẫn chạy; lỗi trả về
  nêu tên từng placement hỏng. `Placements::up_all_report` trả phần đã xong cùng danh sách hỏng (`MigrationReport`,
  `PlacementFailure`). `autoxide migrate up --all` in phần đã xong ra stdout, từng placement hỏng ra stderr, mã thoát
  1. Lượt migrate placement qua API (`POST /api/v1/kernel/placements/migrations`) cũng làm tiếp, kết thúc `failed`
  kèm lỗi thật của từng placement.
- `AUTOXIDE_MIGRATE_ON_START=true` migrate database chính **đồng bộ** trước khi mở port (hỏng thì không khởi động),
  rồi **xếp một lượt `kernel.migrate_placements` chạy nền** cho mọi placement chưa `ready` (`PlacementMigrationQueue`
  của kernel). Không migrate placement đồng bộ lúc khởi động: nhiều khách thì mở port chậm, probe hỏng, máy chủ khác
  chờ khoá migration tới hết hạn rồi khởi động lại liên tục. Khách trên placement chưa xong nhận 503
  `tenant_maintenance` như trước.
- `compose.yml` và `deploy/kubernetes/autoxide.yaml` chạy `migrate up --all` (đồng bộ, một lần mỗi lần deploy); tài liệu
  cập nhật hướng dẫn `--all` (ở chế độ một khách nó giống `migrate up`).
- Mọi đường migrate placement dùng chung khoá và chỗ `active`: D-20261126-placement-migration-exclusive.

Lý do: bước 71 nhóm G3 (D-20261126-road-to-1-0) — nâng cấp phải là một bước. Trước đây khách có database riêng kẹt
`behind` sau mỗi lần nâng nếu người vận hành quên `--all`, và một khách hỏng chặn mọi khách đứng sau theo tên, dù lỗi
chỉ là của riêng khách đó. Mỗi database tự nhớ phần đã xong, nên làm tiếp không gây mất nhất quán; chạy lại vẫn bỏ qua
phần đã áp dụng. Bản đầu của quyết định này migrate mọi placement đồng bộ lúc khởi động; kiểm độc lập chỉ ra nó chặn mở
port khi nhiều khách, nên đổi sang job nền trước khi merge.

Đã cân nhắc: để `serve` từ chối khởi động khi có placement hỏng — khách trên `main` và các placement tốt bị kéo sập
theo một khách; giữ dừng ở lỗi đầu tiên — các khách sau phải chờ vô cớ; migrate placement đồng bộ lúc khởi động — xem
lý do; đổi kiểu trả về của `up_all` — phá vỡ chỗ gọi đang `unwrap()` (testkit, test của module) mà không được gì.

Thay: phần "`AUTOXIDE_MIGRATE_ON_START=true`: áp dụng migration còn chờ của database chính (placement của chế độ
`multi` vẫn cần `autoxide migrate up --all`)" của D-20261121-migrate-on-start
