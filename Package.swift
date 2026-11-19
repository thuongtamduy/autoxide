// swift-tools-version:5.9
// Autoxide Swift SDK (`sdk/swift`): types and client generated at build time from
// `sdk/swift/Sources/Autoxide/openapi.json` (a copy of `docs/openapi.json`, kept in step by
// `cargo xtask sdk`) by Apple's swift-openapi-generator. This manifest sits at the root of the
// repository because Swift Package Manager adds a package from a git URL only from its root.
import PackageDescription

let package = Package(
    name: "Autoxide",
    platforms: [.macOS(.v13), .iOS(.v16), .tvOS(.v16), .watchOS(.v9), .visionOS(.v1)],
    products: [
        .library(name: "Autoxide", targets: ["Autoxide"]),
    ],
    dependencies: [
        .package(url: "https://github.com/apple/swift-openapi-generator", exact: "1.14.4"),
        .package(url: "https://github.com/apple/swift-openapi-runtime", exact: "1.13.4"),
        .package(url: "https://github.com/apple/swift-openapi-urlsession", exact: "1.4.0"),
    ],
    targets: [
        .target(
            name: "Autoxide",
            dependencies: [
                .product(name: "OpenAPIRuntime", package: "swift-openapi-runtime"),
                .product(name: "OpenAPIURLSession", package: "swift-openapi-urlsession"),
            ],
            plugins: [
                .plugin(name: "OpenAPIGenerator", package: "swift-openapi-generator"),
            ],
            path: "sdk/swift/Sources/Autoxide"
        ),
    ]
)
