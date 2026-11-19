# Autoxide Swift SDK

A typed client for the Autoxide HTTP API for iOS, iPadOS, macOS, watchOS, tvOS and visionOS apps.
Apple's [swift-openapi-generator](https://github.com/apple/swift-openapi-generator) builds the types
and the client from `Sources/Autoxide/openapi.json` (a copy of `docs/openapi.json`, kept in step by
`cargo xtask sdk`), so the SDK always matches the server of the same version.

Vietnamese: [`README.vi.md`](README.vi.md).

## Getting it

The package is not on a package index. Add it to your project from this repository, with the tag
of your server's version:

```swift
.package(url: "https://github.com/<owner>/autoxide", exact: "X.Y.Z")
// …and in your target:
.product(name: "Autoxide", package: "autoxide")
```

Xcode: File → Add Package Dependencies…, enter the repository URL and pick the version (the
manifest `Package.swift` is at the root of the repository; the sources are in `sdk/swift`).

## Use

```swift
import Autoxide
import OpenAPIURLSession

let client = Client(
    serverURL: URL(string: "https://api.example.com")!,
    transport: URLSessionTransport()
)
```

Send the access token in `Authorization: Bearer …` (a `ClientMiddleware` that sets the header), and an
`Idempotency-Key` header on writes you may retry. Every error answer is a `Problem` with a stable
`code`. The mobile guide ([`docs/guides/mobile.md`](../../docs/guides/mobile.md)) covers sign-in,
offline sync, events and push notifications.
