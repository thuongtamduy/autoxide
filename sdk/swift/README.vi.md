# SDK Swift của Autoxide

Client có kiểu cho HTTP API của Autoxide, dùng trong app iOS, iPadOS, macOS, watchOS, tvOS và
visionOS. [swift-openapi-generator](https://github.com/apple/swift-openapi-generator) của Apple dựng
kiểu và client từ `Sources/Autoxide/openapi.json` (bản chép của `docs/openapi.json`, `cargo xtask sdk`
giữ cho khớp), nên SDK luôn khớp máy chủ cùng phiên bản.

Tiếng Anh: [`README.md`](README.md).

## Lấy về

Gói không có trên kho gói nào. Thêm vào dự án từ repo này, với tag đúng phiên bản máy chủ:

```swift
.package(url: "https://github.com/<owner>/autoxide", exact: "X.Y.Z")
// …và trong target:
.product(name: "Autoxide", package: "autoxide")
```

Xcode: File → Add Package Dependencies…, nhập URL của repo và chọn phiên bản (file khai báo
`Package.swift` ở gốc repo; mã nguồn ở `sdk/swift`).

## Dùng

```swift
import Autoxide
import OpenAPIURLSession

let client = Client(
    serverURL: URL(string: "https://api.example.com")!,
    transport: URLSessionTransport()
)
```

Gửi access token trong `Authorization: Bearer …` (một `ClientMiddleware` đặt header này), và header
`Idempotency-Key` cho lệnh ghi có thể gửi lại. Mọi câu trả lời lỗi là một `Problem` có `code` ổn
định. Hướng dẫn cho app di động ([`docs/guides/mobile.vi.md`](../../docs/guides/mobile.vi.md)) nói về
đăng nhập, đồng bộ khi mất mạng, nhận event và thông báo đẩy.
