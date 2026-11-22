## D-20261122-base-blocks · Khối nền của `content`, chân trang pháp lý `compliance.legal_footer` tự gắn ngoài bố cục — 2026-11-22 — agent quyết

Chủ đề: Module nội dung, file, liên lạc

Quyết định (bước 67 mục "khối nền", chi tiết hoá D-229 và bảng khối của `docs/steps/67-page-builder.md`):

1. `content` khai 18 khối nền: bố cục `content.section`, `content.columns` (2–4 khối con, mỗi khối con một cột),
   `content.spacer`, `content.divider`; chữ `content.heading`, `content.text`, `content.button`, `content.faq`;
   media `content.image`, `content.gallery`, `content.video`, `content.hero`; thu khách `content.announcement_bar`,
   `content.countdown`, `content.popup`; nhúng `content.embed`, `content.search_box`, `content.placement`. Màu chỉ
   là token của theme (`primary`…), ảnh / video là ID `media`, link theo luật link của thân bài.
2. Khối chữ là **`content.text` chữ thuần** (dòng trống tách đoạn), không phải `rich_text` chứa tài liệu ProseMirror
   như bảng phác thảo: schema khối của kernel chỉ nhận object đóng có kê thuộc tính (D-20261122-block-registry),
   chữ thuần không có đường nào thành HTML, và trang cần chữ có định dạng vẫn dùng thân `rich` của bài.
3. Luật mà schema con không nói được nằm ở bộ kiểm bố cục của `content`, gom lỗi cùng lần (D-083): video có đúng
   một nguồn và `embed_url` chỉ từ host của `AUTOXIDE_CONTENT_EMBED_HOSTS` (cùng hàm kiểm với node `embed`);
   cột 2–4; `content.embed` qua kiểm bản ghi của sổ đối tượng như node `embed` (D-123), và đọc công khai trả nó
   trong `embeds` như thân `rich`.
4. Popup thu email luôn bắt buộc `consent_text` và `policy_link` (bước 37, Luật 91/2025/QH15 — cần đối chiếu văn
   bản gốc); địa chỉ đi tới `newsletter` (chỗ ghi đồng ý), popup không tự lưu gì.
5. Chân trang pháp lý là khối **`compliance.legal_footer`** (luật tên `<module>.<khối>`), dữ liệu = `GET
   /public/owner` + `GET /public/badge` của website đang xem. `content` **tự gắn** nó: `GET /public/resolve` của
   trang bố cục trả `legal_footer` ngoài `body` (cùng lần gọi `BlockData` với khối khác); đặt nó trong bố cục bị từ
   chối (`not_allowed`) — không ai xoá hay đặt trùng được. Website chưa có hồ sơ pháp lý → không có `legal_footer`.
   Khung chung của website (`GET /public/site`, mục sau của bước 67) sẽ trả cùng dữ liệu cho mọi trang.
6. Module `media` không khai khối riêng: `content.image` / `content.gallery` / `content.video` đã là "thư viện ảnh"
   theo ID `media`, và đọc công khai đã trả chi tiết ảnh trong `body_media`. Khối của `forms`, `newsletter`,
   `comments`, `menus` làm ở PR sau của cùng mục.
7. Để sau, theo mục cần chúng: `content.logo` (bộ giao diện), `content.shared_section` (khu dùng chung),
   `content.type_list` / `content.entry_field` (loại nội dung), `content.posts` (danh sách bài cần đọc theo lô một
   lần cho nhiều chuyên mục — làm cùng `content.type_list`).

Lý do: đủ khối để người và AI dựng trang đích thật mà vẫn "từ chối mặc định"; chân trang pháp lý nằm ngoài bố cục
là cách đơn giản nhất để chắc nó luôn có (NĐ 52/2013 sửa bởi NĐ 85/2021, Luật Thương mại điện tử 122/2025 — cần
đối chiếu văn bản gốc / người có chuyên môn pháp lý).

Đã cân nhắc: chèn khối chân trang vào cuối `body` lúc đọc công khai (giao diện khó phân biệt với khối của trang,
và trang tự đặt khối đó thì trùng); `rich_text` mang ProseMirror (cần từ khoá schema riêng cho tài liệu lồng,
kernel phải biết bộ kiểm của `content`); khối `media.gallery` riêng (trùng việc với `content.gallery`).
