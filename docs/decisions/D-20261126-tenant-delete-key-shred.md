## D-20261126-tenant-delete-key-shred · Xoá dữ liệu khách: huỷ khoá dữ liệu, trừ khoá của dữ liệu phải lưu giữ — 2026-11-26 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: Khi xoá dữ liệu của một khách, kernel xoá mọi DEK của khách đó trong `kernel_data_keys` (crypto-shredding:
không còn khoá thì bản mã trong mọi bản sao lưu cũng không đọc được), **trừ** DEK của purpose thuộc bảng mà module khai
phải lưu giữ (`Module::retained_tables`: sổ của `ledger`, hồ sơ của `compliance`). Các DEK đó giữ tới hết hạn lưu của
bảng, rồi xoá cùng dữ liệu. Huỷ khoá như vậy là có chủ ý, không phải mất mát. Xoá bản ghi theo yêu cầu của chủ thể dữ
liệu không đổi: xoá giá trị và blind index của người đó, giữ DEK chung của khách.

Lý do: Bản sao lưu cũ vẫn chứa dữ liệu của khách đã xoá, và không xoá từng dòng trong bản sao lưu được. Xoá khoá là cách
duy nhất làm dữ liệu nhạy cảm trong bản sao lưu không đọc được nữa. Nhưng chứng từ, sổ kế toán phải lưu đủ thời hạn
theo Luật Kế toán (D-190; cần đối chiếu văn bản gốc / người có chuyên môn pháp lý), nên khoá của chúng không được huỷ
sớm. `retained_tables` đã là chỗ module khai nghĩa vụ lưu giữ, dùng lại nó để không có hai danh sách lệch nhau.

Đã cân nhắc: Giữ mọi DEK tới khi bản sao lưu cũ nhất hết hạn (bản sao lưu vẫn đọc được trong thời gian đó, trái mục đích
xoá). Xoá mọi DEK kể cả của dữ liệu phải lưu giữ (vi phạm nghĩa vụ lưu chứng từ). Một DEK cho mỗi chủ thể dữ liệu để xoá
từng người bằng cách xoá khoá (nhiều khoá, nhiều lần mở khoá; để sau, chỉ cho bảng lớn như hồ sơ KYC, theo #577 §2.7).
