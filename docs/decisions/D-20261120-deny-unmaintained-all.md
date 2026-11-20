## D-20261120-deny-unmaintained-all · Ghim `unmaintained = "all"` cho cargo-deny và khôi phục ngoại lệ RUSTSEC-2026-0542 — 2026-11-20 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. `deny.toml` đặt `unmaintained = "all"`: thư viện bị bỏ hoang bị báo cả khi chỉ là phụ thuộc gián tiếp.
2. Khôi phục dòng ngoại lệ RUSTSEC-2026-0542 (`humantime`) mà D-346 đã chấp nhận; lý do ghi rõ `object_store` 0.14.2 (bản
   mới nhất) bắt buộc dùng `humantime`, không có bản nâng nào để chọn.

Lý do: pull request #221 gỡ dòng 0542 vì `cargo deny` trên máy tác giả không còn báo nó. Đó là do khác phiên bản
cargo-deny: bản mới mặc định chỉ tính phụ thuộc trực tiếp của workspace, bản CI cài thì tính cả gián tiếp, nên `gate /
lint` trên CI đỏ ("humantime is unmaintained") dù gate trên máy xanh. Ghim `"all"` làm hai nơi cho cùng kết quả. Đã kiểm
cục bộ: với `"all"` và dòng ngoại lệ thì `cargo deny check -D advisory-not-detected` xanh; thiếu dòng thì đỏ.

Đã cân nhắc:
- Chỉ thêm lại dòng ngoại lệ, không ghim: gate trên máy (bản mới) sẽ đỏ vì `advisory-not-detected`.
- Thay `object_store`: không bản nào thoát `humantime`, và `humantime` cũng chỉ là hệ quả của `chrono` (D-346).

Hệ quả: khi `chrono` được thay (D-346), gỡ cả ba dòng ngoại lệ; gate đỏ báo đúng lúc đó.
