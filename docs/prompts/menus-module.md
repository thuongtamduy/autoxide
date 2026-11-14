# Đề bài: module `menus`, menu của website tách khỏi `content`

Đề bài cho teammate. Nội dung gồm: ghi quyết định, cập nhật lộ trình, viết đặc tả, rồi làm module theo đúng quy
trình (skill `next-step`, gate đầy đủ trên cả SQLite và Postgres). Báo cáo bằng tiếng Việt, lời thường.

## Chủ dự án quyết (14/11/2026)

Menu của website (thanh danh mục, bảng xổ lớn, menu chân trang, menu phụ) **tách thành một module riêng, độc lập**,
vì menu trỏ tới bản ghi của mọi module khác: bài viết, trang, danh mục nội dung, danh mục sản phẩm, chương trình
khuyến mại, gian hàng, cửa hàng. Đặt menu trong `content` là sai chỗ.

Việc của bạn:
- Ghi quyết định này vào `docs/DECISIONS.md` bằng skill `record-decision`, lấy **số kế tiếp còn trống** lúc bạn ghi.
  D-202 và số ngay sau nó đã dành cho `docs/prompts/ui-api-gaps.md` và `docs/prompts/ui-help-api.md`; xem các file
  đó.
- Đánh dấu dòng tiêu đề là "chủ dự án quyết".
- Ghi rõ quyết định này thay phần menu của `content` (bước 17: D-096, D-102, D-105 cho thiết kế và endpoint
  menu; D-149 cho luật từ chối link `//host`, giữ luật này ở module mới).
- Thêm `menus` vào danh sách module chủ dự án đã giao trong `CLAUDE.md` (mục "Hệ quả cho agent").

**Không nhầm với "sổ điều hướng" của bước 51** (`Module::navigation`). Sổ đó là menu của **trang quản trị** (phân
hệ, quyền, gói); module `menus` là menu **trên website** do người quản trị dựng. Hai thứ không gộp.

## Thiết kế đã chọn (cách 3 "kết hợp", chủ dự án đồng ý hướng này)

Menu do người quản trị dựng (toàn quyền thứ tự và lối vào đặc biệt), nhưng mục menu có thể trỏ tới bản ghi của
module khác, và có thể tự lấy nhóm con. Giao diện tham chiếu: thanh danh mục và bảng xổ của bản thiết kế mặt tiền
(`docs/research/ui-design.md` mục 8, `docs/research/storefront-benchmark.md`).

### 1. Kernel: mở rộng sổ đối tượng (D-123)

Thêm vào sổ đối tượng, theo D-038: hàm mới có cài đặt mặc định, module cũ không phải sửa.

- `Resource` thêm `image: Option<Uuid>` (ID media). Bước 51 P6 đã định làm; làm chung một lần.
- `ResourceResolver` thêm hàm tuỳ chọn `children(tenant, id, locale, limit) -> Vec<ResourceRef>`, mặc định trả
  rỗng. Loại nào có cây (danh mục) thì cài.
- Module đăng ký thêm các loại sau, mỗi loại trong bước của nó:
  - `content.term` (danh mục, thẻ của nội dung): làm ngay, vì `content` đã chạy;
  - `catalog.category` (bước 41), có `children`;
  - `promotions.promotion` (43);
  - `marketplace.seller` (46);
  - cửa hàng công khai của bước 34.

  Ghi các mục đó vào lộ trình của từng bước.

### 2. Module `menus`

- **Bảng** (mọi bảng `menus_…`):
  - Menu: `id` (UUIDv7), `tenant_id`, `org_id` (website hay nút sở hữu, theo H1 của
    `docs/prompts/ui-trace-roadmap.md`), `key` (vd `main`, `footer`), `locale`, `version`, thời điểm.
  - Mục menu: `id`, `menu_id`, `parent_id`, `position`. Đích là **một trong ba**:
    - `url` (`https://`, `mailto:`, đường dẫn `/…`; từ chối `//host`);
    - `ref_kind` + `ref_id` (loại và ID trong sổ đối tượng);
    - nhóm tiêu đề không có đích (cột trong bảng xổ).
  - Mục menu còn có:
    - `label` (bỏ trống thì lấy tiêu đề của đối tượng);
    - `auto_children` (bool, chỉ cho đích có `children`) và `auto_children_limit`;
    - trình bày:
      - `icon` (tên trong một bộ biểu tượng cố định);
      - `badge_text` (theo ngôn ngữ) và `badge_tone` (tập cố định: `sale`, `brand`, `new`, `neutral`);
      - `badge_from`, `badge_until` (nhãn có hạn: hết hạn thì tự ẩn);
      - `highlight` (tập cố định);
      - `feature_kind` + `feature_id` (ô "Gợi ý" trong bảng xổ, trỏ tới một đối tượng).
  - Giới hạn như `content` hiện có: tối đa 4 cấp, 200 mục mỗi menu.
  - Không foreign key xuyên module, chỉ giữ ID.
- **API quản trị** (`/api/v1/menus`):
  - danh sách, đọc, tạo;
  - thay cả cây bằng `PUT` có `If-Match` (giống `content`);
  - xoá.
  - Lỗi khi lưu:
    - `ref_kind` chưa đăng ký → `menus.ref_unknown_kind`;
    - đối tượng không tồn tại → `menus.ref_not_found`;
    - `auto_children` trên loại không có cây → `menus.children_unsupported`.
  - Service gom mọi lỗi để báo một lần (D-083).
  - Permission `menus.read`, `menus.manage`.
  - Tool MCP gọi cùng service.
- **API công khai** `GET /api/v1/menus/public/{key}?locale&site`: trả cây đã giải.
  - Mỗi mục có `label`, `href`, `icon`, `badge` (đã lọc theo hạn), `highlight`, `children`, `feature` (tiêu đề, tóm
    tắt, đường dẫn, ảnh).
  - Mục trỏ tới đối tượng không còn công khai thì bỏ, kèm mục con.
  - `auto_children` mở bằng `children()` của sổ đối tượng, có giới hạn.
  - Menu thiếu ngôn ngữ thì dùng ngôn ngữ mặc định (giống `content`).
  - Có ETag và cache. Ghi rõ cách làm mới cache khi đối tượng đổi tên hay đổi đường dẫn: thời hạn cache ngắn, hoặc
    nghe event; chọn cách đơn giản, đủ đúng.
- **Không làm trong module:** khối "Bán chạy" của bảng xổ. Giao diện gọi danh sách sản phẩm công khai của `catalog`
  (lọc theo danh mục, sắp `best_selling`, bước 41) khi người dùng mở bảng xổ. Ghi rõ điều này trong đặc tả.

### 3. Chuyển từ menu của `content` (không phá người đang dùng)

Endpoint menu của `content` đã phát hành, nên không sửa hành vi cũ (D-059):
- Khai `/api/v1/content/menus…` và `/api/v1/content/public/menus/{key}` qua `Module::deprecations`, kèm ngày sunset
  và endpoint thay thế.
- Có đường chuyển dữ liệu không mất menu cũ. Gợi ý: `content` công bố một trait đọc menu (trong
  `autoxide_content::api`); `menus` có lệnh nhập (CLI hoặc endpoint, chạy được nhiều lần không nhân đôi) để nhập
  menu cũ. Mục trỏ tới bài viết thành `ref_kind = content.entry`, mục trỏ tới danh mục hay thẻ thành `content.term`.
  `menus` không đọc bảng của `content`.
- Hết hạn sunset thì gỡ phần menu khỏi `content` bằng migration mới (không sửa migration đã commit).

### 4. Lộ trình và đặc tả

- Viết đặc tả `docs/steps/<số>-menus.md` theo mẫu các bước khác: bảng, API, lỗi và bản dịch, permission, tool
  MCP, event, test, mục "làm theo thứ tự".
- Thêm bước vào `docs/ROADMAP.md`, đặt sau phần sổ đối tượng của bước 51 (P6) và trước bước 41. `catalog.category`
  khai `children` ngay trong bước 41.
- Test chạy trên cả hai database:
  - giải đối tượng;
  - bỏ đối tượng không còn công khai;
  - `auto_children` có giới hạn;
  - nhãn hết hạn tự ẩn;
  - `If-Match` đồng thời (hai người cùng sửa, một người nhận 412);
  - nhập menu cũ hai lần không nhân đôi;
  - thiếu ngôn ngữ thì dùng ngôn ngữ mặc định.

## Báo cáo khi xong

Báo cáo bằng tiếng Việt, lời thường, cho chủ dự án mới học Rust, gồm:
- số quyết định đã ghi;
- bước mới ở đâu trong lộ trình;
- đã làm tới đâu;
- gate có xanh trên cả SQLite và Postgres không;
- việc giao diện cần sửa theo API mới.

Commit trên nhánh làm việc, không push khi chủ dự án chưa yêu cầu.
