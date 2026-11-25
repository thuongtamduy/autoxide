# Đồng bộ nhân viên từ hệ thống danh tính (SCIM 2.0)

Bản tiếng Anh: [`scim.md`](scim.md). Ví dụ dùng `curl` với `API` là máy chủ của bạn (`export API=https://api.example.com`). Chúng chạy như test ở mọi bản dựng, nên khớp với máy chủ của phiên bản này.

Doanh nghiệp quản lý nhân viên ở Microsoft Entra ID, Okta hay Google Workspace có thể để hệ thống đó quản lý tài khoản Autoxide qua [SCIM 2.0](https://www.rfc-editor.org/rfc/rfc7644): người được thêm ở đó có tài khoản nhân sự ở đây; người nghỉ việc bị khoá ngay — phiên đăng nhập, refresh token và API key của họ bị thu hồi trong cùng một bước, không cần ai nhớ xoá tay.

Endpoint SCIM là `/api/v1/auth/scim/v2` (`Users`, `Groups`, `ServiceProviderConfig`, `ResourceTypes`, `Schemas`). Nó theo chuẩn SCIM, không theo quy ước thường của Autoxide: thân là `application/scim+json`, danh sách là `ListResponse` với `startIndex` / `count`, lỗi theo lược đồ lỗi SCIM (`status`, `scimType`, `detail`).

## 1. Tạo token SCIM

Cần permission `auth.scim.manage`, gói có tính năng `auth.scim`, và một phiên đăng nhập: API key không tạo được (`auth.session_required`). Token thuộc về khách, chỉ dùng được ở endpoint SCIM, và chỉ hiện một lần.

**Token chỉ làm được những gì người tạo nó làm được vào lúc này.** Nó tạo được tài khoản, nhưng chỉ cấp, gỡ, khoá hay xoá trong phạm vi quyền hiện tại của người đó: tài khoản có permission mà người đó không có thì token không khoá hay xoá được, và role của nhóm mà người đó không tự cấp được thì token không cấp hay gỡ được (`403`). Hãy tạo token từ tài khoản giữ mọi role mà luật ánh xạ nhóm cấp — thường là quản trị viên.

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
```

<!-- example: create-token -->
```bash
curl -sS "$API/api/v1/auth/scim/tokens" -H "Authorization: Bearer $TOKEN" \
  --json '{"name":"Microsoft Entra ID"}'
# expect: 201
# save: SCIM=.data.token_secret
```

Response có thêm `scim_base_path` (`/api/v1/auth/scim/v2`): ghép địa chỉ công khai của máy chủ vào trước nó khi khai ở hệ thống danh tính. Xem các token bằng `GET /api/v1/auth/scim/tokens` (không bao giờ kèm secret). Xoay vòng: tạo token mới, đổi ở hệ thống danh tính, rồi `DELETE /api/v1/auth/scim/tokens/{id}` token cũ. Mỗi khách có tối đa 5 token.

**Xoay token khi người tạo nghỉ việc hay bị hạ quyền.** Không thì việc bỏ người khỏi nhóm đã có luật và khoá người nghỉ việc hỏng âm thầm ở hệ thống danh tính (nó nhận `403`). `GET /api/v1/auth/scim/tokens` có `creator_status` cho từng token: `disabled`, `deleted` hay `missing_permission` (không còn `auth.scim.manage`) nghĩa là phải xoay ngay. Người tạo chỉ bị hạ role thì hiện `active` nhưng token bị giới hạn theo role mới của họ: khi đó cũng nên xoay.

## 2. Cấu hình hệ thống danh tính

- **Microsoft Entra ID** — *Enterprise applications → New application → Create your own application* (không có trong gallery). Ở *Provisioning*, chọn *Automatic*; *Tenant URL* `https://api.example.com/api/v1/auth/scim/v2`, *Secret Token* là token SCIM; bấm *Test Connection*. Ở *Mappings*, giữ `userName` ← `userPrincipalName`, ánh xạ `emails[type eq "work"].value` ← `mail`, và nên đặt `externalId` ← `objectId`. Entra khoá người bằng `PATCH active=false` và xoá họ sau.
- **Okta** — mở app integration, *General → Provisioning: SCIM*. Ở *Provisioning → Integration*: *SCIM connector base URL* như trên, *Unique identifier field for users* là `userName`, chọn *Push New Users*, *Push Profile Updates* và *Push Groups*, *Authentication Mode* là *HTTP Header* với token SCIM. Ở *To App*, bật *Create Users*, *Update User Attributes* và *Deactivate Users*.
- **Google Workspace** — tự động cấp tài khoản chỉ tới được các ứng dụng trong danh mục của Google. Khi Autoxide chưa có trong danh mục, dùng một cầu nối SCIM đọc từ Google Workspace, hoặc cho nhân viên đăng nhập bằng Google ([`oidc.vi.md`](oidc.vi.md)) và gán role cho tài khoản mới ở đó.

Xem máy chủ hỗ trợ những gì (không cần token):

<!-- example: service-provider-config -->
```bash
curl -sS "$API/api/v1/auth/scim/v2/ServiceProviderConfig"
```

## 3. Mỗi lệnh làm gì

| Lệnh của hệ thống danh tính | Ở Autoxide |
| --- | --- |
| `GET /Users?filter=userName eq "…"` | Tìm tài khoản đã liên kết (`userName` không phân biệt hoa thường; `externalId` có phân biệt). |
| `POST /Users` | Tạo tài khoản nhân sự không mật khẩu — người đó đăng nhập qua nhà cung cấp đăng nhập của bạn hay đặt mật khẩu bằng link đặt lại. Tài khoản nhân sự có sẵn cùng email, chưa liên kết, thì được nhận về thay vì tạo bản trùng. |
| `PUT` / `PATCH /Users/{id}` | Đổi `userName`, `externalId`, tên và email công việc (coi là đã xác minh). Không bao giờ đổi mật khẩu hay đăng nhập hai bước. |
| `active: false` | Khoá tài khoản và, trong cùng transaction, thu hồi mọi phiên, refresh token và API key. Bật lại không làm API key sống lại. Không khoá được quản trị viên cuối cùng còn hoạt động (409). |
| `DELETE /Users/{id}` | Xoá tài khoản như mọi lần xoá tài khoản; các module khác xoá phần dữ liệu họ giữ về người đó. |

Ví dụ các lệnh, như hệ thống danh tính gọi:

<!-- example: create-user -->
```bash
curl -sS "$API/api/v1/auth/scim/v2/Users" -H "Authorization: Bearer $SCIM" \
  --json '{"schemas":["urn:ietf:params:scim:schemas:core:2.0:User"],"userName":"grace@example.com","externalId":"8f2c1e","name":{"givenName":"Grace","familyName":"Hopper"},"emails":[{"value":"grace@example.com","type":"work","primary":true}],"active":true}'
# expect: 201
# save: USER=.id
```

<!-- example: find-user -->
```bash
curl -sS "$API/api/v1/auth/scim/v2/Users?filter=userName%20eq%20%22Grace%40example.com%22" -H "Authorization: Bearer $SCIM"
```

<!-- example: disable-user -->
```bash
curl -sS -X PATCH "$API/api/v1/auth/scim/v2/Users/$USER" -H "Authorization: Bearer $SCIM" \
  --json '{"schemas":["urn:ietf:params:scim:api:messages:2.0:PatchOp"],"Operations":[{"op":"replace","path":"active","value":false}]}'
```

Mọi thay đổi có trong nhật ký audit với loại người làm `scim`.

## 4. Nhóm và role

Hệ thống danh tính cũng đẩy được nhóm của nó (`/Groups`; Okta: *Push Groups*, Entra ID: gán nhóm cho ứng dụng). Nhóm tự nó không cho gì: khách quyết định ý nghĩa của nó bằng **luật ánh xạ** — "thành viên nhóm X giữ role R ở nút N".

1. Để hệ thống danh tính đẩy nhóm sang, rồi tìm id của nó bằng `GET /api/v1/auth/scim/groups` (số thành viên, số luật). Nhóm chưa có luật cũng hiện trong hộp việc (`auth.scim_unmapped_groups`).
2. Thêm luật bằng `POST /api/v1/auth/scim/group-mappings` với `{"group_id": "…", "org_id": "…", "role_id": "…"}` (`org_id` là id của khách cho nút gốc). Bạn phải tự gán được role đó ở nút đó. Thành viên hiện có nhận role ngay; response liệt kê người đang giữ role ở `granted_user_ids`, và người không nhận được (vd gói đã hết chỗ nhân sự) ở `failed` kèm mã lỗi — họ nhận role ở lần đồng bộ thành công sau. **Xem thành viên của nhóm trước khi ánh xạ nó tới role mạnh:** khi nhóm chưa có luật, ai giữ token SCIM cũng thêm được người vào nhóm.
3. Từ đó, người được thêm vào hay bỏ khỏi nhóm có hay mất role ngay — nếu người tạo token SCIM cấp được role đó; không thì thay đổi bị từ chối (`403`) và không có gì được ghi. Role ai đó được gán tay không bao giờ bị SCIM gỡ, và role hai nhóm cùng cấp vẫn còn khi người đó còn ở một trong hai nhóm.

Gỡ luật bằng `DELETE /api/v1/auth/scim/group-mappings/{id}` (bạn phải tự cấp được role của luật, không thì `auth.role_beyond_you`); xoá nhóm ở hệ thống danh tính thì luật của nó và role nó đã cấp cũng bị gỡ.

## 5. Hỗ trợ những gì

- Bộ lọc: `eq`, `ne`, `co`, `sw`, `ew`, `pr`, nối bằng `and` / `or` và ngoặc. User theo `userName`, `externalId`, `emails`, `active`, `id`. Thứ khác trả `400` với `scimType` `invalidFilter`.
- `PATCH` (RFC 7644 §3.5.2): `add`, `replace`, `remove`, với đường dẫn như `active`, `name.givenName`, `emails[type eq "work"].value`, hay không đường dẫn. Thuộc tính Autoxide không lưu (phần mở rộng enterprise, số điện thoại, chức danh…) bị bỏ qua.
- Phân trang: `startIndex` (từ 1) và `count` (tối đa 200).
- Không hỗ trợ: bulk, sắp xếp, ETag, đổi mật khẩu.

Lỗi: `401` khi không có token SCIM hợp lệ, `403` khi gói không có `auth.scim` hay thay đổi vượt quyền của người tạo token (không có `scimType`), `404` với id của khách khác, `409` `uniqueness` khi `userName`, `externalId` hay email thuộc tài khoản khác, `400` `invalidValue` / `invalidSyntax` / `invalidPath` / `noTarget` / `mutability` khi thân sai.

## Dữ liệu cá nhân

Khách là bên kiểm soát dữ liệu nhân viên của mình và Autoxide xử lý theo chỉ dẫn của khách (Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15 — cần đối chiếu văn bản gốc với người có chuyên môn pháp lý). Chỉ các thuộc tính kể trên được lưu; liên kết SCIM có trong bản xuất dữ liệu của tài khoản và bị xoá cùng tài khoản.
