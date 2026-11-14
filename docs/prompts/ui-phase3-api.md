# Đề bài: lấp chỗ trống API và đặc tả mà các màn hình đợt 3 lộ ra

Đề bài cho teammate làm phần backend: đặc tả trong `docs/steps/`, lộ trình và quyết định.

Đợt 3 của thiết kế đã vẽ các phần sau:
- **Website:**
  - phía người mua: tải hàng số, tranh chấp, dùng điểm, mã người tiếp thị, trả góp, giỏ chỉ có hàng số;
  - đánh giá và hỏi đáp có lọc và nút hữu ích;
  - trang livestream công khai;
  - ba trang chiến dịch.
- **Quản trị:**
  - trình soạn bài và thư viện media theo khung 2;
  - ô lọc bằng lời thường;
  - báo cáo kênh bán.

Xem `docs/research/ui-traceability.md`, mục "Đợt 3". Mỗi màn chỉ dùng thứ đặc tả đã có. Chỗ nào màn cần mà đặc tả chưa có thì ghi ở đây.

Đề bài này **bổ sung** cho `docs/prompts/ui-trace-roadmap.md` và `docs/prompts/ui-phase2-api.md`, không lặp lại. Mục đã có ở hai đề bài đó thì làm theo đề bài đó. Ví dụ:
- `org_id` cho nội dung;
- lọc `missing_alt`, `ai_generated`;
- lưu mã giảm giá;
- nhắc flash sale;
- bản nháp tóm tắt đánh giá bằng AI;
- lý do ẩn đánh giá.

## Cách làm

1. Với mỗi mục, chọn một trong ba hướng, rồi chọn cách đơn giản nhất mà giải quyết trọn vấn đề:
   - thêm vào đặc tả của bước;
   - ghi vào phần "việc để sau" của lộ trình;
   - bác bỏ, kèm lý do.
2. Quyết định có ảnh hưởng lâu dài thì ghi bằng skill `record-decision`.
3. Giữ luật `CLAUDE.md`, nhất là:
   - chạy trên cả SQLite và Postgres;
   - module sở hữu bảng của mình, không đọc bảng của module khác;
   - quy định hay đổi là dữ liệu có ngày hiệu lực;
   - căn cứ chưa chắc thì ghi "cần đối chiếu văn bản gốc";
   - core không giữ tiền người mua (D-187).
4. Không viết code ở đề bài này.

Ký hiệu: **thiếu** là không có ở đâu; **hở** là đặc tả dùng mà chưa định nghĩa; **lệch** là hai chỗ mâu thuẫn.

## Việc theo bước

### Người mua: hàng số, tranh chấp, điểm, tiếp thị, trả góp (41, 45, 46, 47, 49, 36)

| Mục | Bước | Loại | Nội dung | Màn |
|---|---|---|---|---|
| Quyền tải hàng số của tôi | 41, 45 | thiếu | Thêm `GET /me/downloads`: mỗi file có số lượt còn, hạn link, đơn gốc. | Tài khoản › Tải hàng số |
| Gửi lại link tải | 45 | thiếu | Thêm `POST /me/downloads/{id}/resend`. Token không lưu dạng đọc lại được, nên lệnh này phát token mới, làm link cũ hết hiệu lực và giữ nguyên số lượt. | Tài khoản › Tải hàng số |
| Giới hạn và đổi trả hàng số | 45 | hở | Ghi rõ: số lượng tối đa mỗi dòng (màn chặn ở 1 bản), quy tắc đổi trả cho hàng số. | Thanh toán |
| Thuế GTGT của hàng số | 36, 41 | hở | Sách điện tử, khoá học online có chịu GTGT không, mức bao nhiêu. Màn đang để 8%. Cần đối chiếu Luật 48/2024. | Thanh toán |
| Tranh chấp phía người mua | 46 | thiếu | Thêm `GET /me/disputes` (hiện `GET /disputes` chỉ cho sàn và người bán). Thêm chuyển trạng thái do người mua làm: đồng ý đề nghị → `resolved`, nhờ sàn phân xử → `escalated`, rút → `withdrawn`. Chốt `platform_due_at` tính từ lúc nào; màn tính 7 ngày từ lúc đẩy lên sàn. | Tài khoản › Tranh chấp |
| Mặc định của dùng điểm | 47 | hở | Giá trị mặc định cho `min_redeem_points` và `max_redeem_ratio_bp`; màn dùng 100 điểm và 30%. Ghi rõ điểm có dùng cho hàng của người bán trên sàn không; màn chỉ cho dùng với hàng của cửa hàng. Liên quan mục dòng tiền nhiều chế độ ở đề bài trước. | Thanh toán |
| Kiểm mã người tiếp thị ở giỏ | 49 | thiếu | Endpoint công khai kiểm một mã người tiếp thị khi người mua tự nhập. Hiện `POST /public/clicks` chỉ nhận mã của link, nên mã sai bị loại im lặng sau khi đặt. Người tự mua bằng mã của mình thì không ghi nhận. | Thanh toán |
| Bảng trả góp của đối tác | 36, 45 | thiếu | Thêm vào `PaymentGateway` một hàm lấy điều kiện và bảng kỳ hạn của đối tác theo số tiền: kỳ hạn, phí, tiền mỗi kỳ, mức tối thiểu và tối đa, giờ báo giá. Ghi rõ trả góp có áp khi giỏ có hàng của người bán trên sàn không, và hạn duyệt của đối tác. Cần đối chiếu quy định cho vay tiêu dùng. | Thanh toán |

### Đánh giá và hỏi đáp sản phẩm (45, D-202 phần A)

| Mục | Loại | Nội dung | Màn |
|---|---|---|---|
| Lọc và sắp xếp đánh giá | hở | Danh sách công khai lọc theo số sao, có ảnh, biến thể (cỡ). Sắp theo hữu ích, mới nhất, điểm thấp, điểm cao. Phân trang bằng con trỏ. | Sản phẩm › Đánh giá |
| Nút "Hữu ích" | thiếu | Cho đánh giá và cho câu trả lời trong hỏi đáp. Mỗi tài khoản một phiếu, bấm lại thì bỏ. Có số đếm, và dùng số này để sắp "hữu ích nhất". Chịu được gọi lại (idempotency). | Sản phẩm |
| Chủ đề người mua hay nhắc | thiếu | Ví dụ "giữ nhiệt tốt 48", "tay cầm nóng 12". Lấy từ cùng bản nháp tóm tắt AI chờ duyệt (đề bài trước). Mỗi chủ đề có số đếm và lọc được. Chỉ hiện sau khi nhân viên duyệt (D-109). | Sản phẩm › Đánh giá |
| Báo vi phạm từ người mua | thiếu | Endpoint công khai báo một đánh giá, câu hỏi hay phiên livestream, có danh sách lý do. Đi vào hàng chờ của `compliance`. Hiện đặc tả chỉ nói báo vi phạm đến từ `compliance`. | Sản phẩm, Livestream |
| Tên người mua rút gọn | hở | Khi hiện công khai, rút tên thành dạng "Hoài Thương N.". Ghi thành quy ước chung cho đánh giá, hỏi đáp, bình luận. Cần đối chiếu Luật 91/2025. | Sản phẩm |

### Livestream (49)

| Mục | Loại | Nội dung |
|---|---|---|
| Thông tin công khai của phiên | hở | `GET /public/live-sessions/{id}` cần ghi rõ có trả: hồ sơ công khai của người dẫn và người bán (tên hiển thị, nhãn đã xác thực), link bản ghi lại, thời lượng. |
| Nhắc trước giờ phát | thiếu | Đặt nhắc qua `notifications`, cần đồng ý riêng theo bước 37. |
| Ghim sản phẩm trước giờ phát | hở | Có ghim được trước giờ phát không. Màn vẽ là "hiện khi phiên bắt đầu". |
| Lưu vết phiên | hở | Thời hạn lưu vết phiên. Cần đối chiếu Luật Thương mại điện tử 2025. |
| Bình luận và tin nhắn trực tiếp | ghi chú | Không có trong đặc tả nên không vẽ. Giữ nguyên trừ khi chủ dự án mở. |

### Trang chiến dịch và hàng hoá (41, 43, content)

| Mục | Bước | Loại | Nội dung |
|---|---|---|---|
| Trang chiến dịch, trang một sản phẩm | content, 51 | thiếu | Cơ chế dựng trang riêng gom nội dung, sản phẩm, mã và khung giờ flash sale. Có thể là một loại mục của `content` nhúng bản ghi của module khác qua sổ đối tượng ở bước 51. |
| Dữ liệu công khai cho trang chiến dịch | 43 | thiếu | Endpoint gom mã của chương trình, khung giờ flash sale và số suất còn. |
| Số đã bán trên thẻ sản phẩm | 41 | thiếu | Hiện chỉ có cột sắp `best_selling`. Cần trường số đã bán (làm tròn: 1,2k) nếu muốn hiện. |
| Đặt lịch tư vấn tại cửa hàng | — | thiếu | Không thuộc bước nào. Có thể làm bằng `forms` có xác nhận và đồng ý dữ liệu cá nhân; ghi thời hạn giữ số điện thoại. |

### Nội dung và media (16, 17, 34, 37, 51)

| Mục | Bước | Loại | Nội dung | Màn |
|---|---|---|---|---|
| Alt của tệp media theo ngôn ngữ | 16 | thiếu | `media.alt` chỉ có một ngôn ngữ. Cần alt theo locale ở chính tệp, hoặc một quy ước. | Thư viện media |
| Chỗ đang dùng một tệp | 16, 51 | thiếu | Thêm `GET /media/{id}/usages`. `media` không biết ai dùng mình, nên các module báo qua sổ đối tượng hay trait (D-123). | Thư viện media |
| Đánh dấu phần do AI gợi ý | 17, 37 | hở | API có `ai_generated` / `ai_reviewed` ở cấp mục. Màn cần đánh dấu phần nào do AI gợi ý (vd mô tả SEO). Cần đối chiếu Luật 134/2025 và nghị định hướng dẫn. | Soạn bài |
| Link xem trước công khai | 17 | ghi chú | API chỉ trả `token`; địa chỉ trang xem trước là việc của repo giao diện. Ghi rõ trong đặc tả. | Soạn bài |

### Lọc bằng lời thường (51, 52) và báo cáo kênh (48)

| Mục | Bước | Loại | Nội dung |
|---|---|---|---|
| Toán tử "một trong nhiều" | 51 P7 | hở | Schema bộ lọc cần `status=a,b`. Ví dụ "COD chưa giao" phải ghép `payment_status=cod_pending` với nhiều giá trị `status`. |
| Trả về của `POST /ai/filters/parse` | 52 | hở | Trả các tham số đã kiểm theo schema của danh sách, kèm phần câu chưa hiểu. Không bao giờ tự chạy thao tác (D-109, D-201). |
| Trạng thái "Chờ xác nhận" | 45 | lệch | Màn Đơn hàng dùng trạng thái này (theo H2 của đề bài trước) nhưng bảng trạng thái của bước 45 chưa có. |
| Cách tính của báo cáo kênh | 48 | hở | `GET /reports/channels` cần chốt ba điều: phí và thực nhận tính theo kỳ bảng kê hay theo ngày đặt (màn chọn: doanh thu theo ngày đặt, phí theo bảng kê có kỳ nằm trong khoảng); có trả số đơn huỷ hoặc trả không; có tách kênh tự có (website, quầy) với kênh ngoài không. |

### Từ đợt sửa theo nghiên cứu (xem `docs/research/storefront-benchmark.md`)

| Mục | Bước | Loại | Nội dung |
|---|---|---|---|
| Thuộc tính đồ bếp có cấu trúc | 41 | thiếu | Các thuộc tính cùng nhãn và cùng đơn vị cho cả ngành hàng: số người ăn theo cỡ, "trong hộp có gì" và "không kèm", hướng dẫn dùng và chăm sóc (kèm tệp PDF), kích thước và khối lượng kiện. |
| Quan hệ "Dùng kèm" | 41 | thiếu | Hàng bổ sung có giá theo biến thể (vd nắp theo cỡ chảo). Tách khỏi "hàng thay thế" (Có thể bạn thích). |
| Mã áp cho biến thể nào | 43 | hở | Điều kiện của mã trả về số lượt còn và danh sách biến thể được áp, để trang tự ẩn mã ở cỡ không áp. |
| Đánh giá theo người bán | 45, 46 | thiếu | Endpoint gộp đánh giá mọi sản phẩm của một người bán: phân bố sao, số có ảnh theo từng mức sao, lọc, sắp xếp, phân trang bằng con trỏ, lượt hữu ích. |
| Tóm tắt trang "Tôi" | 47, 34 | thiếu | Một endpoint trả số liệu tài khoản (đơn chưa xong, lượt tải còn, điểm, bảo hành còn hạn, tranh chấp đang mở), để khỏi gọi 9 API khi mở trang. |
| Xác nhận đơn qua SMS | 45, `notifications` | hở | Email không còn bắt buộc khi đặt hàng giao tận nơi (theo khuyến nghị giảm số ô phải nhập). Đặc tả việc gửi xác nhận đơn qua SMS khi khách không có email. Phần tra đơn bằng mã đơn và số điện thoại đã có. |
| Hoá đơn công ty cần email | 36 | hở | Ghi rõ: xuất hoá đơn cho công ty thì bắt buộc email nhận hoá đơn. |
| Người mua xác nhận đã nhận hàng | 45 | thiếu | `POST /me/orders/{id}/confirm-receipt`, gọi lại được (idempotency key). Mở hạn đổi trả và nút đánh giá; không thay việc hãng vận chuyển báo đã giao. |
| Đổi trả kèm ảnh, video mở hộp | 45 | thiếu | Yêu cầu đổi trả nhận `media_ids` (ảnh, video qua module `media`). Đơn trả thêm `return_deadline_at`, `returnable` và `return_blocked_reason` (mã ổn định, có bản dịch), để giao diện tắt nút kèm lý do thay vì ẩn nút. |
| Thưởng điểm khi đánh giá | 47 | thiếu | Mức thưởng là dữ liệu có ngày hiệu lực, giao diện ghi sẵn trên nút ("+20 điểm"). Thưởng không phụ thuộc số sao hay nội dung khen chê, vì thưởng theo đánh giá tốt là đánh giá có điều kiện. Cần đối chiếu văn bản gốc về đánh giá có thưởng. |
| Ngày giao dự kiến dạng khoảng | 44 | hở | `ShippingOption` trả `eta_from` và `eta_to` (ngày theo giờ Việt Nam). Giao diện tự viết "Thứ Bảy 14/11 – Chủ nhật 15/11", điện thoại viết gọn hơn. Không trả câu đã viết sẵn. |
| Hỏi đáp: sắp xếp và hữu ích | 45 (D-202 phần A) | hở | Danh sách câu hỏi nhận `order=helpful\|newest`. Lượt "hữu ích" một người một lần, cùng cách với đánh giá. |
| Hàng thay thế ("Có thể bạn thích") | 41 | thiếu | Quan hệ `alternatives` giữa sản phẩm, kèm một dòng "khác ở điểm nào" theo ngôn ngữ. Tách khỏi "Dùng kèm". |
| "Hôm nay nấu gì?" | `content`, 41 | thiếu | Công thức là bài viết; dụng cụ cần dùng gắn qua khối nhúng trỏ tới `catalog.product` trong sổ đối tượng (D-123). Không chép ID sản phẩm vào thân bài. Trả cả bộ dụng cụ cùng giá và tồn hiện tại. |
| Thêm nhiều dòng vào giỏ một lần | 45 | thiếu | `POST /cart/lines:batch` gọi lại được (idempotency key). Dòng hết hàng báo riêng từng dòng, không làm hỏng cả lô. |
| Sắp theo mức giảm | 41, 43 | thiếu | Danh sách sản phẩm công khai nhận `order=discount_desc`, tính theo giá thật ngay trước khuyến mại. |

### Từ rà soát trang quản trị (R4, 14/11/2026)

| Mục | Bước | Loại | Nội dung |
|---|---|---|---|
| Menu con của phân hệ do core trả về | 51 (`Module::navigation`) | hở | Sổ điều hướng trả cho mỗi phân hệ danh sách màn (nhãn, đường dẫn, thứ tự cố định), lọc theo permission và gói. Giao diện chỉ vẽ; mọi màn trong cùng phân hệ dùng một danh sách. |
| Số đếm trên menu và thanh bên | 51, 52 (hộp việc) | thiếu | Mỗi mục menu có thể kèm một số "việc đang chờ" và cờ gấp, lấy cùng nguồn với hộp việc. Hiện các màn tự ghi số nên lệch nhau (đơn 37 / 12 / 412; COD 1 / 2). |
| Lỗi theo từng ô | chung (D-083) | đã có | Giao diện đã đánh dấu ô sai, ghi lỗi dưới từng ô và đưa con trỏ về ô sai đầu tiên. Cần `Problem` trả danh sách lỗi theo tên trường (`errors[].field`). Kiểm mọi endpoint ghi đều theo đúng cách này. |
| Hoàn tác | chung | hở | Thao tác đảo được (hoãn việc, đổi trạng thái, ẩn bình luận) cần endpoint đảo lại, hoặc trả kèm "lệnh đảo" để giao diện gọi khi người dùng bấm Hoàn tác trong 6 giây. |

## Căn cứ cần đối chiếu (agent đã ghi trên màn)

- Mức giảm tối đa 50% và trường hợp chương trình khuyến mại tập trung quốc gia: Thông tư 39/2025/TT-BCT, NĐ 81/2018 sửa bởi NĐ 128/2024.
- Người tiếp thị phải xác thực danh tính: Luật Thương mại điện tử 2025.
- Thời hạn giữ dữ liệu khi đặt lịch tư vấn: Luật 91/2025.
- GTGT của hàng số và dịch vụ đào tạo: Luật 48/2024.
- Livestream trên điện thoại: phần công khai thông tin được thu gọn sau một nút, dòng tóm tắt (ai bán, hoa hồng
  của người dẫn, đối tác thanh toán) luôn hiện. Cần đối chiếu xem cách này đã đủ yêu cầu công khai chưa.
- Trang sản phẩm: chi tiết điều kiện khuyến mại nằm trong mục mở rộng, còn phạm vi và hạn luôn hiện. Cần đối chiếu
  yêu cầu công khai thông tin khuyến mại.

## Báo cáo khi xong

Báo cáo bằng tiếng Việt, lời thường, cho chủ dự án mới học Rust. Nội dung báo cáo:
- quyết định đã ghi;
- mục nào vào đặc tả hay lộ trình, ở bước mấy;
- mục nào bác bỏ và vì sao;
- chỗ nào còn "cần đối chiếu văn bản gốc";
- gate có xanh không.

Commit trên nhánh làm việc, không push khi chủ dự án chưa yêu cầu.
