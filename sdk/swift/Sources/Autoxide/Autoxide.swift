// `Client` và các kiểu của API do plugin swift-openapi-generator sinh lúc build từ `openapi.json`
// cạnh file này (bản chép `docs/openapi.json`). File này chỉ để target có mã nguồn Swift của riêng nó.

/// Facts about this SDK.
public enum AutoxideSDK {
    /// The API contract the client was generated from, inside this package.
    public static let contract = "openapi.json"
}
