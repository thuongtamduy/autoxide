## D-20261119-relocation-switch-hook · Hook đồng bộ ở bước chuyển của lượt tách / gộp khách — 2026-11-19 — agent quyết

Chủ đề: Đa khách, tổ chức, gói

Quyết định: Kernel thêm trait `tenancy::SwitchHook`, module gắn vào chính sách tách / gộp của mình bằng
`Relocation::on_switch(hook)`. Không thêm hàm mới vào trait `Module`: `Module::relocation()` đã có, mặc định không
có hook (D-038). Mọi hook chạy ở bước `switch` (`verified → switched`), trong **cùng transaction** ở database điều
khiển với việc đổi khách mới sang `active`, và **trước** việc đó. Hook nhận `RelocationSwitch {kind, source,
target}` và transaction. Hook lỗi thì transaction bỏ, khách mới vẫn `provisioning`, lượt đứng ở `verified` và chạy
lại được. Vì vậy hook phải chạy lại được. `auth` dùng nó để chép rào quyền sang khách mới khi tách
(D-20261119-step54-guardrails). Event `kernel.relocation_started` giữ làm lưới an toàn.

Lý do: Chính sách bảo mật đặt theo ID khách ở database điều khiển (rào quyền) phải có hiệu lực từ request đầu tiên
tới khách mới. Event chỉ được giao khi khách nguồn phục vụ lại, nên có một khe để khách mới chạy mà không bị rào: đó
là lỗ leo quyền. Chạy trong cùng transaction thì không có trạng thái nửa vời. Gắn hook vào `Relocation` thay vì thêm
hàm vào `Module`: hook đi cùng chính sách tách / gộp của module, kernel đã gom chính sách đó sau `start`, nên module
có sẵn service đã dựng.

Đã cân nhắc: (1) Hàm mới `Module::on_relocation_prepare`, như phiên giữ tenancy gợi ý. Bị loại vì cùng hiệu quả
nhưng thêm một hàm vào trait, và kernel phải giữ danh sách module ở chỗ chạy lượt tách. (2) Hook chạy ở bước `copy`
(database đích). Bị loại vì rào nằm ở database điều khiển, và bước `copy` chạy lại khi lỗi trước khi khách mới mở.
