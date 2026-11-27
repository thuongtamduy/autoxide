## D-20261126-tenant-delete-key-shred · Xoá dữ liệu khách: huỷ khoá dữ liệu, trừ khoá của dữ liệu phải lưu giữ — 2026-11-26 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: Khoá có ba tầng: KEK của bản cài → tenant key (một khoá mỗi khách, trong KMS hay trong kho khoá riêng
ngoài database và ngoài file backup của database) → DEK (mỗi khách và purpose, bọc bằng tenant key, lưu trong
`kernel_data_keys`). Khi xoá dữ liệu của một khách, kernel **huỷ tenant key** của khách đó (crypto-shredding): bản backup
database cũ còn DEK đã bọc nhưng không còn khoá để mở. Trước khi huỷ, DEK của purpose thuộc bảng mà module khai phải lưu
giữ (`Module::retained_tables`: sổ của `ledger`, hồ sơ của `compliance`) được bọc lại bằng **retention key** riêng của
khách, giữ tới hết hạn lưu của bảng rồi huỷ cùng dữ liệu. Huỷ khoá như vậy là có chủ ý, không phải mất mát. Xoá bản ghi
theo yêu cầu của chủ thể dữ liệu không đổi: xoá giá trị và blind index của người đó, giữ khoá chung của khách.

Lý do: Bản sao lưu cũ vẫn chứa dữ liệu của khách đã xoá, và không xoá từng dòng trong bản sao lưu được. Xoá khoá là cách
duy nhất làm dữ liệu nhạy cảm trong bản sao lưu không đọc được nữa — nhưng chỉ khi khoá bị xoá **không** nằm trong chính
bản sao lưu đó: DEK bọc nằm trong database, KEK dùng chung cho cả bản cài, nên phải có tầng tenant key ở ngoài database. Nhưng chứng từ, sổ kế toán phải lưu đủ thời hạn
theo Luật Kế toán (D-190; cần đối chiếu văn bản gốc / người có chuyên môn pháp lý), nên khoá của chúng không được huỷ
sớm. `retained_tables` đã là chỗ module khai nghĩa vụ lưu giữ, dùng lại nó để không có hai danh sách lệch nhau.

Đã cân nhắc: Xoá DEK đã bọc trong database (bản backup cũ còn DEK, KEK dùng chung vẫn mở được — không huỷ được gì).
Giữ mọi DEK tới khi bản sao lưu cũ nhất hết hạn (bản sao lưu vẫn đọc được trong thời gian đó, trái mục đích
xoá). Xoá mọi DEK kể cả của dữ liệu phải lưu giữ (vi phạm nghĩa vụ lưu chứng từ). Một DEK cho mỗi chủ thể dữ liệu để xoá
từng người bằng cách xoá khoá (nhiều khoá, nhiều lần mở khoá; để sau, chỉ cho bảng lớn như hồ sơ KYC, theo #577 §2.7).
