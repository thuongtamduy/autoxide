# Rà pháp lý vòng 1 — 26/11/2026

[English (tóm tắt)](README.en.md)

> **Báo cáo này do AI thực hiện, không phải ý kiến pháp lý.** Một phiên AI đóng vai nhóm hai người độc lập với đội
> viết code — luật sư công nghệ (dữ liệu cá nhân, thương mại điện tử, an ninh mạng) và chuyên gia thuế, hoá đơn
> điện tử — theo quyết định của chủ dự án giao AI đóng vai rà soát thay chuyên gia bên ngoài (bước 71, mục G12). Căn cứ được tra trên web; phần lớn nguồn tìm được là
> bản tin của công ty luật, báo chí và trang tư vấn, **không phải văn bản gốc**. Mọi kết luận cần người có chuyên môn
> pháp lý đối chiếu văn bản gốc trước khi dựa vào (D-186). Không có kết luận nào ở đây thay cho tư vấn của luật sư.

Phạm vi: core và module trong repo trên `main` ngày 26/11/2026 (commit `9b7d1a6de`), nhìn từ phía người triển khai
Autoxide cho một khách thật ở Việt Nam. Không sửa code trong vòng này; việc sửa ghi thành mục `[ ]` ở G12 của
`docs/ROADMAP.md` và giao phiên khác.

## Mức

| Mức | Nghĩa |
| --- | --- |
| **chặn 1.0** | Sai luật hiện hành hoặc tài liệu công khai hứa điều sai, có thể gây vi phạm ngay khi khách dùng theo hướng dẫn. Phải sửa trước 1.0. |
| **cần sửa** | Thiếu hay lệch so với quy định; người triển khai phải tự bù. Nên sửa trước 1.0, ít nhất phải ghi rõ trong tài liệu. |
| **khuyến nghị** | Làm thì tốt hơn; không thấy quy định buộc ngay. |
| **thông tin** | Đã kiểm, thấy ổn hoặc chỉ cần theo dõi. |

## Tóm tắt

- **2 điểm chặn 1.0:** căn cứ thương mại điện tử đã hết hiệu lực (NĐ 52/2013 và NĐ 85/2021 bị NĐ 248/2026 thay từ
  01/7/2026) nằm trong danh sách kiểm và tài liệu (LG-1); ngày lập hoá đơn và ngày tính thuế lấy theo UTC, nên đơn đặt
  từ 00:00 tới 07:00 giờ Việt Nam ngày 01/01/2027 sẽ còn bị tính GTGT 8% đã hết hạn (LG-2).
- **11 điểm cần sửa**, gồm: định danh người bán chưa nối vào đâu (LG-3), thời hạn trả lời yêu cầu của chủ thể dữ liệu
  dựa trên nghị định đã bị thay (LG-4), không có gì cho dữ liệu trẻ em (LG-5), AI gửi dữ liệu cá nhân ra nước ngoài mà
  không vào sổ xử lý (LG-6), gói thuế chưa theo cải cách thuế hộ kinh doanh 2026 (LG-7), luồng khấu trừ thay của sàn
  chưa nối (LG-8), huỷ hoá đơn qua SePay luôn lỗi và không có thông báo sai sót (LG-9), hoá đơn tính lại thuế khác đơn
  (LG-10), hạn mức khuyến mại dẫn điều đã bị bãi bỏ (LG-11), tài liệu hứa ẩn đánh giá qua `ModerationPolicy` nhưng
  code không làm (LG-12), căn cứ an ninh mạng và lưu dữ liệu trong nước cũ (LG-13).
- **7 khuyến nghị** và **4 điểm thông tin** (phần đã làm đúng: sổ đồng ý, sổ tay sự cố 72 giờ, analytics không cookie,
  dòng tiền không giữ tiền người mua).

## Bảng phát hiện

### Thương mại điện tử

#### LG-1 · Căn cứ thương mại điện tử đã hết hiệu lực — **chặn 1.0**

- **Căn cứ:** Luật Thương mại điện tử 122/2025/QH15, hiệu lực 01/7/2026
  ([thuvienphapluat](https://thuvienphapluat.vn/van-ban/Thuong-mai/Luat-Thuong-mai-dien-tu-2025-so-122-2025-QH15-662035.aspx)).
  Nghị định 248/2026/NĐ-CP (ký 30/6/2026, hiệu lực 01/7/2026) hướng dẫn Luật, theo bản tổng quan **thay thế NĐ
  52/2013/NĐ-CP và NĐ 85/2021/NĐ-CP**; Điều 23–30 về thông báo / đăng ký nền tảng trước khi hoạt động; Điều 4–17 về
  thông tin phải công khai (gồm tiêu chí tìm kiếm, xếp hạng; quy tắc điểm thưởng; giá đã gồm thuế, phí vận chuyển
  chưa); Điều 22 báo cáo định kỳ (năm: trước 15/02); Điều 41 lưu thông tin sản phẩm ít nhất 1 năm, dữ liệu hợp đồng ít
  nhất 3 năm; Điều 53 chuyển tiếp: nền tảng đã thông báo / đăng ký trước 01/7/2026 làm lại hồ sơ tới 30/6/2027
  ([hethongphapluat — tổng quan NĐ 248](https://hethongphapluat.com/nghi-dinh-248-2026-nd-cp-huong-dan-luat-thuong-mai-dien-tu.html),
  [Bộ Công Thương phổ biến NĐ 248](https://moit.gov.vn/tin-tuc/bo-cong-thuong-pho-bien-luat-thuong-mai-dien-tu-va-nghi-dinh-so-248-2026-nd-cp.html)).
  Một nguồn nói thông báo nền tảng bán hàng trực tiếp do UBND cấp tỉnh xác nhận — **cần đối chiếu văn bản gốc**
  (cơ quan tiếp nhận, còn dùng online.gov.vn không, mẫu logo).
- **Hiện trạng:** `modules/compliance/rules/vn.json:135-136` (`moit.checklist.sales` / `.platform`) chỉ có version 1,
  `effective_from` 2022-01-01, căn cứ NĐ 52/2013 sửa bởi NĐ 85/2021; không có phiên bản từ 01/7/2026. `vn.json:93`
  (`cases.deadlines`) dẫn NĐ 85/2021. Tài liệu công khai `docs/COMPLIANCE.md:35`, `docs/COMPLIANCE.vi.md:35` và `:277`
  nói "NĐ 52/2013 sửa bởi NĐ 85/2021" như luật đang áp dụng. Danh sách kiểm thiếu các mục mới (tiêu chí xếp hạng, quy
  tắc điểm, báo cáo định kỳ, thời hạn chuyển tiếp).
- **Đề xuất:** thêm phiên bản 2 của hai quy tắc `moit.checklist.*` và `cases.deadlines`, hiệu lực 01/7/2026, theo NĐ
  248/2026 (danh mục trang, mục mới, `legal_basis` từng điều); ghi thời hạn chuyển tiếp 30/6/2027 vào quy tắc; sửa
  `COMPLIANCE.md` / `.vi.md` và mẫu trang của `content`. Không cần đổi code nếu danh sách kiểm chỉ là dữ liệu — mục kiểu
  "fact" mới (vd tiêu chí xếp hạng) thì chưa module nào trả lời, sẽ hiện `unknown` / `manual`.

#### LG-3 · Định danh người bán, người livestream, tiếp thị liên kết chưa chặn ở đâu — **cần sửa**

- **Căn cứ:** Luật TMĐT 122/2025/QH15 (định danh người bán; theo quy tắc của repo là Điều 17 — cần đối chiếu); NĐ
  248/2026 Điều 18 (thông tin định danh người bán trong nước / nước ngoài), theo bản tổng quan **áp dụng từ 01/01/2027**
  (Điều 52); Điều 12, 19 (quy chế livestream, định danh điện tử người livestream).
- **Hiện trạng:** quy tắc `kyc.required_activities` (`vn.json:7`) bật `sell`, `livestream`, `affiliate`. Nhưng
  `KycGate::require_verified` (`modules/compliance/src/kyc.rs:881`, `provider.rs:28`) chỉ được gọi trong test; cổng
  `ListingGate` của `catalog` (`modules/catalog/src/api.rs:516`) không có ai cài nên luôn cho đăng bán. Module
  `marketplace`, livestream, `affiliate` chưa có (dừng tới sau 1.0). `docs/COMPLIANCE.md:18` liệt kê "KYC người bán,
  người livestream, người tiếp thị liên kết" như tính năng có sẵn. Thông tin định danh mà NĐ 248 Điều 18 đòi (ngày
  sinh, số định danh cá nhân; hộ chiếu còn hạn ít nhất 6 tháng cho người nước ngoài) cần đối chiếu với trường của hồ sơ
  KYC.
- **Đề xuất:** (1) tài liệu ghi rõ: core có hồ sơ định danh và cổng, **chưa chặn đăng bán** cho tới khi module sàn cài
  `ListingGate` gọi `KycGate`; (2) mục "cài cổng KYC vào `catalog` và luồng nhận đơn của người bán" đặt làm điều kiện
  của module sàn; (3) đối chiếu trường hồ sơ KYC với Điều 18 NĐ 248 trước 01/01/2027.

#### LG-11 · Hạn mức khuyến mại dẫn điều đã bị bãi bỏ — **cần sửa**

- **Căn cứ:** NĐ 239/2026/NĐ-CP (ký 26/6/2026, hiệu lực từ ngày ký) sửa NĐ 81/2018: **bãi bỏ Điều 6 (hạn mức giá trị
  hàng khuyến mại) và Điều 7 (mức giảm giá tối đa)**, giao Bộ Công Thương quy định; hồ sơ đăng ký khuyến mại mới
  ([PwC](https://www.pwc.com/vn/vn/publications/news-brief/260716-decree-239-promotion-schemes.html),
  [KPMG](https://kpmg.com/vn/vi/insights/2026/07/decree-239-on-sales-promotion.html),
  [LuatVietnam](https://luatvietnam.vn/doanh-nghiep/diem-moi-ve-xuc-tien-thuong-mai-tai-nghi-dinh-239-2026-nd-cp-561-111132-article.html)).
  Thông tư của Bộ Công Thương đặt hạn mức mới — **chưa tra được số hiệu; cần đối chiếu** (gói dữ liệu đang dẫn TT
  39/2025/TT-BCT).
- **Hiện trạng:** `modules/promotions/data/vn/legal_limits.json:20-71` vẫn ghi căn cứ "NĐ 81/2018 Điều 7 (mức giảm tối
  đa 50%)" và "Điều 6" cho mức đang hiệu lực; `notice_rules.json` dẫn NĐ 81/2018 Điều 17 sửa bởi NĐ 128/2024, chưa
  theo thủ tục của NĐ 239/2026. Con số 50% có thể vẫn đúng (nếu thông tư giữ), nhưng căn cứ thì sai. Tài liệu
  `docs/PROMOTIONS.md:37`, `.vi.md:35`.
- **Đề xuất:** tra thông tư của Bộ Công Thương ban hành theo NĐ 239/2026; thêm phiên bản mới của hạn mức và quy tắc
  thông báo / đăng ký có `effective_from` 26/6/2026; sửa `legal_basis` và tài liệu. Code không phải đổi (đã là dữ liệu).

#### LG-12 · Tài liệu hứa ẩn đánh giá qua `ModerationPolicy`, code không làm — **cần sửa**

- **Căn cứ:** Luật Bảo vệ quyền lợi người tiêu dùng 19/2023/QH15 và NĐ 55/2024/NĐ-CP (không được ẩn đánh giá, phản hồi
  của người tiêu dùng trừ lý do luật cho — cần đối chiếu điều cụ thể).
- **Hiện trạng:** `docs/COMPLIANCE.md:338-343` (và bản VI) nói module kiểm duyệt đánh giá hỏi `ModerationPolicy` trước
  khi ẩn. `modules/orders/src/reviews.rs` không gọi trait đó; dùng danh sách lý do riêng
  (`modules/orders/src/reasons.rs:20-28`: `unlawful_content`, `personal_data`, `abusive_language`,
  `spam_or_advertising`, `not_about_product`) khác mã của quy tắc `moderation.hide_reasons` (`vn.json:84-91`). Quy tắc
  có ngày hiệu lực vì vậy không có tác dụng với đánh giá sản phẩm.
- **Đề xuất:** `orders` hỏi `ModerationPolicy` (khi `compliance` bật) với ánh xạ mã lý do rõ ràng, hoặc sửa tài liệu cho
  đúng hiện trạng. Chọn một — không để tài liệu hứa điều code không làm.

### Dữ liệu cá nhân

#### LG-4 · Thời hạn trả lời yêu cầu của chủ thể dữ liệu dựa trên nghị định đã bị thay — **cần sửa**

- **Căn cứ:** Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15 (hiệu lực 01/01/2026) và NĐ 356/2025/NĐ-CP (hiệu lực
  01/01/2026, thay NĐ 13/2023). Theo bản phân tích
  ([Luật Việt An](https://luatvietan.vn/diem-moi-cua-nghi-dinh-356-2025-so-voi-nghi-dinh-13-2023-ve-huong-dan-bao-ve-du-lieu-ca-nhan.html)),
  NĐ 356 Điều 5 đặt thời hạn theo loại yêu cầu: rút đồng ý / hạn chế / phản đối — trả lời trong 2 ngày làm việc, thực
  hiện trong 15 ngày (20 ngày nếu có bên xử lý thứ ba); xem, sửa, cung cấp — 10 ngày; xoá — 20 ngày (dài hơn khi có bên
  thứ ba). Nguồn tự mâu thuẫn ở con số có bên thứ ba — **cần đối chiếu văn bản gốc**.
- **Hiện trạng:** `modules/compliance/rules/vn.json:75-81` (`privacy.subject_requests`) đặt 72 giờ cho mọi loại,
  `legal_basis` tự ghi "72 giờ lấy theo NĐ 13/2023 (đã được thay thế)". Hình dạng quy tắc chỉ có một con số giờ cho mỗi
  loại — không có hạn "trả lời / xác nhận" riêng với hạn "thực hiện", không tính ngày làm việc, không có hạn kéo dài khi
  có bên xử lý.
- **Đề xuất:** quy tắc version 2 theo NĐ 356 Điều 5: mỗi loại có `acknowledge` và `fulfil` (đơn vị ngày làm việc hay
  ngày, dùng lịch ngày làm việc sẵn có của `cases.deadlines`), cờ "có bên xử lý" kéo dài hạn. 72 giờ chặt hơn luật nên
  không vi phạm, nhưng căn cứ sai và thiếu mốc "xác nhận trong 2 ngày làm việc".

#### LG-5 · Không có gì cho dữ liệu cá nhân của trẻ em — **cần sửa**

- **Căn cứ:** Luật 91/2025/QH15 có quy định riêng về dữ liệu cá nhân của trẻ em và người được giám hộ (đồng ý của cha
  mẹ / người giám hộ; với trẻ từ đủ 7 tuổi thêm đồng ý của trẻ — **chưa tra được điều cụ thể; cần đối chiếu văn bản
  gốc**).
- **Hiện trạng:** tìm `age`, `minor`, `child`, `dob`, `ngày sinh`, `trẻ em`, `guardian`… trong repo: không có trường
  ngày sinh, cờ người chưa thành niên, xác nhận tuổi hay bằng chứng đồng ý của người giám hộ. Sổ đồng ý
  (`modules/compliance/src/consents.rs`) chỉ ghi đồng ý của chính chủ thể.
- **Đề xuất:** (1) chính sách của khách "dịch vụ có hướng tới trẻ em không" và bước tự khai tuổi khi đăng ký (cấu hình
  được, mặc định tắt); (2) sổ đồng ý ghi được người đồng ý thay (vai, quan hệ) làm bằng chứng; (3) tài liệu nói rõ khách
  bán hàng cho trẻ em phải bật. Thiết kế cần người có chuyên môn xác nhận.

#### LG-6 · AI và bên xử lý khác gửi dữ liệu cá nhân ra nước ngoài mà không vào sổ xử lý — **cần sửa**

- **Căn cứ:** Luật 91/2025/QH15 (chuyển dữ liệu cá nhân xuyên biên giới, hồ sơ đánh giá tác động chuyển dữ liệu —
  Điều 20, 22 theo các bản tổng hợp, cần đối chiếu); NĐ 356/2025 Điều 18 (hồ sơ Mẫu 09, thông báo Bộ Công an, thoả thuận
  bằng văn bản ràng buộc bên nhận) theo bản phân tích ở LG-4. NĐ 330/2026/NĐ-CP (hiệu lực 19/8/2026) phạt vi phạm về
  dữ liệu cá nhân ([VOV](https://vov.vn/chinh-tri/tu-1982026-tu-y-thu-thap-xu-ly-du-lieu-ca-nhan-bi-phat-toi-70-trieu-dong-post1326424.vov),
  [PwC](https://www.pwc.com/vn/vn/publications/news-brief/260915-decree-330-pdpl-violations.html)).
- **Hiện trạng:** sổ xử lý (`GET /processing-register`, `modules/compliance/src/privacy.rs:62-73,711-730`) chỉ có bên
  nhận do khách tự khai và nhà cung cấp mã theo dõi. Module `ai` (`modules/ai/src/providers.rs`) gửi nội dung tới
  Anthropic / OpenAI / Gemini…; `conversations` gửi 20 tin nhắn gần nhất của khách hàng để soạn nháp
  (`modules/conversations/src/assist.rs:38-39`), `orders` gửi đánh giá để tóm tắt (`modules/orders/src/reviews.rs:56`).
  Không có thông báo, không vào sổ xử lý, `docs/AI.md` không nhắc chuyển dữ liệu ra nước ngoài. Cùng vấn đề với nhà cung
  cấp mail, SMS, lưu trữ file đặt ngoài Việt Nam.
- **Đề xuất:** kernel cho module khai "bên xử lý và nước nhận" theo cấu hình đang chạy (vd `Module::processors`, mặc
  định rỗng — D-038), sổ xử lý gộp tự động; `autoxide doctor` cảnh báo khi có bên nhận ngoài Việt Nam mà khách chưa ghi
  đã lập hồ sơ đánh giá; `docs/AI.md` thêm mục dữ liệu cá nhân. Tối thiểu cho 1.0: tài liệu.

#### LG-13 · Căn cứ an ninh mạng và lưu dữ liệu trong nước cũ — **cần sửa**

- **Căn cứ:** Luật An ninh mạng 116/2025/QH15 (ban hành 10/12/2025, hiệu lực 01/7/2026)
  ([LuatVietnam](https://english.luatvietnam.vn/lawoncybersecurityno116-2025-qh15dateddecember102025ofthenationalassembly-422396-doc1.html)).
  Nghị định hướng dẫn mới (có nguồn nhắc NĐ 333/2026/NĐ-CP) có thể thay NĐ 53/2022/NĐ-CP — **chưa xác nhận; cần đối
  chiếu** quy định lưu dữ liệu tại Việt Nam áp cho ai.
- **Hiện trạng:** `docs/COMPLIANCE.md` / `.vi.md` (mục "Nơi lưu dữ liệu") dẫn NĐ 53/2022; tính năng `data_residency`
  bật theo từng khách, không mặc định.
- **Đề xuất:** tra nghị định hướng dẫn Luật 116/2025; sửa căn cứ trong tài liệu; nếu luật buộc nhóm khách nào lưu
  trong nước thì nêu trong hướng dẫn triển khai (`DEPLOY`).

### Thuế, hoá đơn điện tử

#### LG-2 · Ngày lập hoá đơn và ngày tính thuế theo UTC — **chặn 1.0**

- **Căn cứ:** NĐ 123/2020/NĐ-CP sửa bởi NĐ 70/2025/NĐ-CP (hiệu lực 01/6/2025), Điều 9 (thời điểm lập hoá đơn); Nghị
  quyết 204/2025/QH15 và NĐ 174/2025/NĐ-CP: GTGT 8% **tới hết 31/12/2026**
  ([LuatVietnam EN](https://english.luatvietnam.vn/thue/decree-174-2025-nd-cp-vat-reduction-404331-d1.html),
  [DFDL](https://www.dfdl.com/insights/legal-and-tax-updates/vietnam-extension-of-the-reduced-8-vat-rate-to-31-december-2026/)).
- **Hiện trạng:** `modules/invoices/src/issue.rs:195` lấy ngày lập `Timestamp::now().date_utc()`; `modules/orders/src/pricing.rs:538`
  lấy `tax_point` bằng `input.at.date_utc()`. Tài liệu `docs/TAX.md:54` hứa "ngày theo múi giờ của người bán". Giờ Việt
  Nam là UTC+7: đơn hay hoá đơn từ 00:00 tới 06:59 ngày D mang ngày D−1. Hệ quả cụ thể: từ 00:00 tới 07:00 ngày
  **01/01/2027** (còn 5 tuần) đơn hàng và hoá đơn vẫn tính GTGT 8% đã hết hạn; mỗi ngày, hoá đơn lập trong khung giờ đó
  ghi sai ngày, sai kỳ khai thuế ở ranh giới tháng / quý.
- **Đề xuất:** ngày lập và `tax_point` tính theo múi giờ của người bán (hồ sơ thuế / cấu hình tổ chức, mặc định
  `Asia/Ho_Chi_Minh` cho vùng thuế VN); test ở mốc 31/12/2026 23:30 và 01/01/2027 00:30 giờ Việt Nam trên cả hai
  database. Thay đổi chạm thuế — qua kiểm độc lập (D-20261126-independent-review). Nên xong trước 31/12/2026.

#### LG-7 · Gói thuế chưa theo cải cách thuế hộ kinh doanh 2026 — **cần sửa**

- **Căn cứ:** NĐ 68/2026/NĐ-CP (ký 05/3/2026) về chính sách và quản lý thuế với hộ, cá nhân kinh doanh: Điều 10 (hộ có
  doanh thu năm trên 500 triệu đồng), Điều 11 (khai, khấu trừ qua sàn), Điều 8 (sử dụng hoá đơn); theo các bài tổng hợp:
  hộ doanh thu năm không quá 500 triệu đồng không nộp GTGT, TNCN; từ 1 tỷ đồng phải dùng hoá đơn điện tử
  ([toàn văn — chinhphu.vn](https://xaydungchinhsach.chinhphu.vn/toan-van-nghi-dinh-68-2026-nd-cp-quy-dinh-ve-chinh-sach-thue-quan-ly-thue-voi-ho-kinh-doanh-119260306102906789.htm),
  [thuvienphapluat](https://thuvienphapluat.vn/van-ban/Doanh-nghiep/Nghi-dinh-68-2026-ND-CP-chinh-sach-thue-va-quan-ly-thue-doi-voi-ho-kinh-doanh-ca-nhan-kinh-doanh-685358.aspx)).
  Hộ đã bị sàn khấu trừ mà cả năm dưới ngưỡng được hoàn (NĐ 117/2025). Tỉ lệ khấu trừ của sàn năm 2026 — các nguồn
  **không thống nhất; cần đối chiếu**.
- **Hiện trạng:** `modules/tax/packs/vn.json:28-33` chỉ dẫn NĐ 117/2025, không có NĐ 68/2026; quy tắc
  `einvoice.cash_register_required` (`vn.json:36`) dẫn NĐ 70/2025 với ngưỡng 1 tỷ; không có ngưỡng 500 triệu, không có
  dữ liệu cho hoàn thuế. `TaxEngine::invoice_requirements` không module nào gọi; `orders` luôn dùng hoá đơn máy tính tiền
  cho kênh `pos` (`modules/orders/src/invoicing.rs:253-254`) bất kể chế độ thuế.
- **Đề xuất:** gói `vn` version 2: căn cứ NĐ 68/2026 cho quy tắc hộ kinh doanh, ngưỡng không chịu thuế là dữ liệu có
  ngày hiệu lực, đối chiếu lại tỉ lệ khấu trừ; `orders` hỏi `invoice_requirements` thay vì quyết theo kênh;
  `docs/TAX.md` / `.vi.md` cập nhật.

#### LG-8 · Khấu trừ, khai thay thuế của sàn chưa nối — **cần sửa**

- **Căn cứ:** NĐ 117/2025/NĐ-CP (hiệu lực 01/7/2025) và NĐ 68/2026 Điều 11: sàn có chức năng đặt hàng và thanh toán
  khấu trừ, khai thay, nộp thay GTGT, TNCN cho từng giao dịch của hộ kinh doanh; cấp chứng từ khấu trừ
  ([EY](https://www.ey.com/vi_vn/technical/tax/tax-and-law-updates/tin-nhanh-tu-van-thue-nhan-su-thang-6-nam-2025-nghi-dinh-117-ve-quan-ly-thue-doi-voi-hoat-dong-kinh-doanh-tren-nen-tang-thuong-mai-dien-tu-nen-tang-so)).
- **Hiện trạng:** `ledger` có bút toán `FlowEvent::TaxWithheld` (`modules/ledger/src/flows.rs:599-640`) nhưng không
  module nào phát (chỉ e2e test); loại chứng từ `withholding_certificate` có mà không ai tạo; không có dữ liệu cho tờ
  khai khấu trừ thay. `docs/TAX.md` / `MONEY.md` mô tả khấu trừ như việc core làm.
- **Đề xuất:** gắn với module sàn (sau 1.0): phát `TaxWithheld` khi chi hộ, sinh chứng từ khấu trừ, báo cáo theo kỳ khai
  thay. Cho 1.0: tài liệu nói rõ phần đã có (tính, bút toán) và phần chưa (phát tự động, chứng từ, tờ khai).

#### LG-9 · Huỷ hoá đơn qua SePay luôn lỗi; không có thông báo sai sót — **cần sửa**

- **Căn cứ:** NĐ 123/2020 Điều 19 sửa bởi NĐ 70/2025 (xử lý hoá đơn có sai sót: điều chỉnh, thay thế; thông báo sai sót
  Mẫu 04/SS trong các trường hợp nghị định nêu) — **cần đối chiếu các trường hợp còn được huỷ và còn phải gửi 04/SS sau
  NĐ 70/2025**.
- **Hiện trạng:** `modules/einvoice_sepay/src/provider.rs:227-229` `cancel` luôn trả `not_supported`; `invoices` gọi nhà
  cung cấp trước khi đổi trạng thái (`modules/invoices/src/issue.rs:720-727`), nên không huỷ được hoá đơn đã gửi qua
  SePay. Không có thông báo sai sót 04/SS trong code.
- **Đề xuất:** xác định với chuyên gia hoá đơn: khi nào chỉ được điều chỉnh / thay thế, khi nào huỷ; API trả lỗi rõ
  hướng người dùng sang điều chỉnh / thay thế; nếu 04/SS còn bắt buộc thì làm qua nhà cung cấp.

#### LG-10 · Hoá đơn tính lại thuế, có thể lệch số khách đã trả — **cần sửa**

- **Căn cứ:** NĐ 123/2020 sửa bởi NĐ 70/2025 Điều 9 (thời điểm lập); Luật Thuế GTGT 48/2024/QH15 (thuế suất áp tại thời
  điểm xác định thuế — cần đối chiếu).
- **Hiện trạng:** `modules/orders/src/invoicing.rs:206-283` gửi danh mục thuế của dòng, `invoices` tính lại theo ngày
  lập; mặc định lập khi giao xong (`orders.invoice_timing = on_delivered`). Đơn đặt 30/12/2026 giá gồm 8%, giao
  03/01/2027 → hoá đơn tính 10% trên cùng tổng tiền đã thu, hoặc lệch tổng. Code không báo chênh lệch.
- **Đề xuất:** quyết (cùng chuyên gia thuế) thuế suất của hoá đơn khi đơn qua mốc đổi thuế; tối thiểu phát hiện chênh
  lệch giữa ảnh chụp thuế của đơn và của hoá đơn, báo người dùng thay vì lặng lẽ khác.

## Khuyến nghị

| Mã | Nội dung | Căn cứ | Hiện trạng | Đề xuất |
| --- | --- | --- | --- | --- |
| LG-14 | Hồ sơ đánh giá tác động xử lý (Mẫu 10) và chuyển dữ liệu ra nước ngoài (Mẫu 09) | NĐ 356/2025 (theo bản phân tích ở LG-4); Luật 91/2025: doanh nghiệp nhỏ, khởi nghiệp, hộ kinh doanh, doanh nghiệp siêu nhỏ được miễn một số nghĩa vụ 5 năm, trừ khi xử lý dữ liệu nhạy cảm hay số lượng lớn ([KPMG](https://kpmg.com/vn/vi/home/phan-tich-chuyen-sau/2025/06/vietnam-new-personal-data-protection-law.html)) — cần đối chiếu | `/processing-register` chỉ là đầu vào (`docs/COMPLIANCE.md:213-217`) | Xuất bản nháp theo trường của hai mẫu; ghi rõ ai được miễn |
| LG-15 | Dữ liệu nhạy cảm mở rộng | NĐ 356/2025 coi dữ liệu theo dõi hành vi, thói quen dùng dịch vụ trực tuyến, dữ liệu tài khoản ngân hàng là nhạy cảm (theo bản phân tích) — cần đối chiếu | `DataCategory::Sensitive` chỉ dùng cho KYC và ghi âm; analytics gắn tài khoản (`analytics.personal`), lịch sử mua không xếp nhạy cảm | Rà phân loại theo NĐ 356; email / số điện thoại trong sổ đồng ý đang lưu rõ cạnh bản băm — cân nhắc mã hoá |
| LG-16 | Xác thực tài khoản đăng nội dung mặc định tắt | NĐ 147/2024/NĐ-CP (mạng xã hội chỉ cho tài khoản đã xác thực bằng số điện thoại Việt Nam hay định danh đăng tin, bình luận) — website có bình luận có phải "mạng xã hội" không: cần đối chiếu | `posting.*` mặc định `none` (`modules/compliance/src/verification.rs:200-205`); không có verifier thì mọi người đăng nhập coi như đã xác thực (`crates/kernel/src/security/access/mod.rs:1015-1024`) | `autoxide doctor` cảnh báo khi bật bình luận / đánh giá công khai mà mức xác thực là `none`; tài liệu giải thích |
| LG-17 | Thời hạn lưu và báo cáo của NĐ 248/2026 | Điều 41 (thông tin sản phẩm ≥ 1 năm, hợp đồng ≥ 3 năm, người bán tải về được); Điều 22 (báo cáo năm trước 15/02; gỡ trong 24 giờ, báo trong 48 giờ) | Đơn giữ như chứng từ kế toán (đủ); chưa kiểm `catalog` giữ sản phẩm đã xoá 1 năm; không có báo cáo định kỳ | Kiểm `catalog`; dữ liệu cho báo cáo năm |
| LG-18 | Căn cứ gói GTGT 8% thiếu số hiệu | NQ 204/2025/QH15, NĐ 174/2025/NĐ-CP (phụ lục hàng không được giảm) | `modules/tax/packs/vn.json:19` ghi "nghị quyết của Quốc hội và nghị định" không số | Ghi số hiệu; tài liệu nhắc khách tự xếp hàng không được giảm vào `vat.standard` |
| LG-19 | Sắp có nghị định thay NĐ 91/2020 (tin nhắn rác) | Dự thảo của Bộ Công an (tháng 9–12/2026), [file dự thảo](https://bocongan.gov.vn/media/bca-media/library-20260909084715-f7e1171f-eb0c-4ae2-97ed-7017b136dc65-1-nghi-inh-thay-the-n-91.doc) | Khung giờ, nhãn QC là dữ liệu (`vn.json:118-134`) — tốt; `max_marketing_per_day.sms` là `null` và không code nào đọc | Theo dõi; khi ban hành thì nhập phiên bản quy tắc mới; đọc giới hạn SMS/ngày |
| LG-20 | Năm lưu hoá đơn theo năm dương lịch | Luật Kế toán 88/2015 Điều 41, NĐ 174/2016 Điều 12 (10 năm sau khi kết thúc năm tài chính / kỳ kế toán năm) | `modules/invoices/src/service.rs:123` tính theo năm dương lịch | Dùng năm tài chính của khách khi khác năm dương lịch |

## Thông tin (đã kiểm, thấy ổn)

- **LG-21 · Sổ tay sự cố 72 giờ.** NĐ 356/2025 dùng Mẫu 08 báo vi phạm trong 72 giờ từ khi phát hiện (theo các bản phân
  tích); quy tắc `privacy.incident` (`vn.json:34-40`) và job nhắc khớp. NĐ 330/2026 phạt báo trễ / không đúng — nên ghi
  vào tài liệu.
- **LG-22 · Sổ đồng ý, cookie, analytics.** Đồng ý theo mục đích, có văn bản và phiên bản, rút dễ như cho; mã theo dõi
  chỉ tải sau đồng ý, từ chối ngang đồng ý; analytics không cookie, không lưu IP. Phù hợp hướng "im lặng không là đồng
  ý" mà NĐ 330/2026 phạt.
- **LG-23 · Dòng tiền.** Core không giữ tiền người mua (D-187): ba chế độ qua đối tác, COD không về tài khoản sàn, thẻ
  quà tặng / số dư mặc định tắt, vòng kín (NĐ 52/2024/NĐ-CP — cần đối chiếu cho thẻ trả trước nhiều bên).
- **LG-24 · Lưu chứng từ.** Hoá đơn chỉ xoá được bản nháp; sổ cái chỉ thêm, kỳ đã khoá không mở; đủ cho nghĩa vụ lưu 10
  năm (trừ LG-20).

## Cần người có chuyên môn xác nhận

1. NĐ 248/2026: cơ quan tiếp nhận thông báo website bán hàng (Bộ hay UBND / Sở Công Thương), còn dùng online.gov.vn và
   logo xanh / đỏ không, danh mục thông tin bắt buộc, bán trước khi được xác nhận có bị cấm không (D-358 đang "gắn nhãn,
   không chặn") — LG-1.
2. Trường định danh người bán của NĐ 248 Điều 18 và ngày áp dụng 01/01/2027 — LG-3.
3. Thông tư của Bộ Công Thương đặt hạn mức khuyến mại sau NĐ 239/2026; thủ tục đăng ký / thông báo mới — LG-11.
4. NĐ 356/2025 Điều 5: con số thời hạn từng loại yêu cầu, có bên thứ ba — LG-4.
5. Điều về dữ liệu trẻ em của Luật 91/2025 (độ tuổi, ai đồng ý) — LG-5.
6. Khi nào gửi dữ liệu tới nhà cung cấp AI / mail / lưu trữ nước ngoài là "chuyển dữ liệu xuyên biên giới" cần hồ sơ;
   phạm vi miễn trừ cho doanh nghiệp nhỏ — LG-6, LG-14.
7. Nghị định hướng dẫn Luật An ninh mạng 116/2025 và nghĩa vụ lưu dữ liệu trong nước — LG-13.
8. NĐ 68/2026: ngưỡng 500 triệu, tỉ lệ khấu trừ của sàn năm 2026, ngưỡng hoá đơn máy tính tiền / hoá đơn điện tử — LG-7.
9. Hoá đơn khi đơn qua mốc đổi thuế suất; còn được huỷ hoá đơn không và khi nào gửi 04/SS sau NĐ 70/2025 — LG-9, LG-10.
10. Website có bình luận / đánh giá có thuộc diện NĐ 147/2024 không — LG-16.

## Cách làm vòng này

Đọc `CLAUDE.md` (Luật số hai), D-186, D-187, D-188, D-190, quy tắc `modules/compliance/rules/vn.json`,
`modules/tax/packs/vn.json`, `modules/promotions/data/vn/*.json` và code của `compliance`, `invoices`,
`einvoice_sepay`, `tax`, `ledger`, `orders`, `customers`, `notifications`, `newsletter`, `esms`, `auth`, `analytics`,
`ai`, `conversations`, `catalog`; tài liệu `docs/COMPLIANCE*`, `TAX*`, `MONEY*`, `PROMOTIONS*`, `AI*`. Tra căn cứ bằng
tìm kiếm web ngày 26/11/2026. Vòng sau nên làm với văn bản gốc trong tay (Công báo, vbpl.vn) và kiểm lại các điểm đã
sửa.
