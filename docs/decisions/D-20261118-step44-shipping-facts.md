## D-20261118-step44-shipping-facts · `shipping`: trả lời tự động về vận đơn cho `support`, câu mẫu khai bằng `answer_text!` — 2026-11-18 — agent quyết

Chủ đề: Thương mại

Bối cảnh: đặc tả bước 44 mục 8 và D-230 (trả lời tự động của `support` dựng từ dữ liệu chính xác, câu trả lời là mẫu câu
i18n của module nguồn). `cargo xtask check` chỉ thấy câu `text:` khai qua một số hàm dựng (`Permission::new`,
`EventInfo::new`…), nên câu trả lời viết thành chuỗi trần sẽ bị báo "bản dịch không dùng" và không bị đòi dịch.

Quyết định:

1. `shipping` gắn một `FactSource` vào `SupportFacts` lúc `start` khi có cài `support` (không cài thì bỏ qua). Ý định
   `shipping.tracking`, tham số `tracking_number` / `order_number`.
2. Chỉ trả lời chủ đơn: `Requester::User` trùng `buyer_id` của vận đơn, hay `Requester::VerifiedPhone` trùng số người
   nhận (so theo chữ số, `+84…` = `0…`). Email chưa trả lời (vận đơn không giữ email người mua). Không thấy, nhiều vận
   đơn (giao một phần) hay người lạ → `None`, nhân viên trả lời.
3. Câu trả lời và cụm từ trạng thái bọc trong macro `answer_text!("…")` của module (chỉ trả lại chuỗi); `xtask` nhận
   `answer_text!(` như một chỗ khai câu `text:`, nên câu được đòi bản dịch tiếng Việt. Module sau (vd `orders`) cài
   `FactSource` dùng cùng quy ước.

Hệ quả: người mua hỏi "đơn của tôi đâu" nhận trạng thái thật ngay, bằng ngôn ngữ của họ; không lộ vận đơn cho người
không chứng minh được là chủ đơn.
