## D-20261125-gitleaks-scope · Secret scan chỉ quét lịch sử đi tới được từ HEAD (`--log-opts=HEAD`), không quét mọi nhánh — 2026-11-25 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định: mọi lượt `gitleaks git` quét lịch sử (job `secrets` của `.github/workflows/ci.yml` và bước "secrets
(gitleaks)" của `gate.sh`, nên cả bước lint của `gate.yml`) thêm `--log-opts=HEAD`. Vẫn quét TOÀN BỘ lịch sử đi tới
được từ HEAD (secret đã commit rồi xoá vẫn bị bắt). Trên CI HEAD là ref được checkout: pull request là merge ref
(lịch sử `main` + commit của PR), push lên `main` là `main`; trên máy là HEAD đang đứng. Hai lượt `--pre-commit` của
`gate.sh` giữ nguyên; không đổi rule, không thêm allowlist.

Lý do: không truyền `--log-opts` thì gitleaks (v8.30.1) chạy `git log -p -U0 --full-history --all`, tức quét mọi ref
đã fetch. Một nhánh của phiên khác chưa có PR chứa chuỗi bị `generic-api-key` bắt làm job đỏ trên mọi pull request
khác (và gate trên máy nào đã fetch nhánh đó). Một PR chỉ nên bị chặn bởi những gì nó chứa.

Đã kiểm bằng thực nghiệm (nhánh tạm cục bộ chứa chuỗi giả giống khoá): mặc định từ nhánh sạch → đỏ; với
`--log-opts=HEAD` từ nhánh sạch → xanh, đứng trên nhánh có chuỗi → đỏ; chuỗi commit rồi xoá trong lịch sử HEAD → vẫn đỏ.

Bổ sung cùng quyết định (điều phối yêu cầu): nguyên nhân sự cố là gate không quét file MỚI chưa track — chạy gate xanh
rồi mới `git add` + commit thì chuỗi mẫu lọt (`git` chỉ thấy lịch sử, `--pre-commit` chỉ thấy file đã track). `gate.sh`
quét thêm các file untracked không bị gitignore: `gitleaks dir` chỉ nhận một đường dẫn nên chép danh sách
`git ls-files --others --exclude-standard` (giữ đường dẫn tương đối) vào `.git/gitleaks-untracked` rồi quét thư mục đó.
Kiểm bằng thực nghiệm: file mới chưa track chứa chuỗi giả → đỏ; cùng chuỗi trong đường dẫn gitignore (`target/`) → không quét.

Hệ quả: nhánh chưa có PR không được quét trên CI cho tới khi mở PR (hoặc nó vào `main`); gate trên máy của chính
phiên đó vẫn quét nhánh đang làm. Muốn quét định kỳ mọi nhánh thì cần một job riêng không chặn PR (ghi ở `docs/ROADMAP.md`).
