// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "PocketCore",
    platforms: [.iOS(.v18), .macOS(.v15)],
    products: [
        .library(name: "PocketKit", targets: ["PocketKit"]),
        .library(name: "PocketUI", targets: ["PocketUI"]),
        .library(name: "PocketMLX", targets: ["PocketMLX"]),
    ],
    dependencies: [
        .package(url: "https://github.com/ml-explore/mlx-swift", exact: "0.32.3"),
        .package(url: "https://github.com/ml-explore/mlx-swift-lm", exact: "3.32.3"),
        .package(url: "https://github.com/huggingface/swift-transformers", exact: "1.3.5"),
    ],
    targets: [
        .binaryTarget(name: "pocket_ffiFFI", path: "build/pocket_ffiFFI.xcframework"),
        .target(
            name: "PocketKit",
            dependencies: ["pocket_ffiFFI"],
            path: "swift/Sources/PocketKit",
            linkerSettings: [
                // pocket-trust pulls in reqwest -> hyper-util's system proxy
                // detection, which needs SystemConfiguration on Apple platforms.
                .linkedFramework("SystemConfiguration"),
                // llama-cpp-2 (via pocket-engine) bundles llama.cpp C++ source
                // into libpocket_ffi.a; consumers must link libc++.
                .linkedLibrary("c++"),
                // llama.cpp's Metal backend references these at the final
                // executable link (cargo's framework link directives don't
                // propagate out of the static lib into Xcode's app link).
                .linkedFramework("Metal"),
                .linkedFramework("MetalKit"),
                .linkedFramework("Accelerate"),
                .linkedFramework("Foundation"),
            ]
        ),
        .target(
            name: "PocketUI",
            path: "swift/Sources/PocketUI",
            resources: [.process("Resources/Colors.xcassets"), .process("Resources/MenuBar.xcassets")]
        ),
        .target(
            name: "PocketMLX",
            dependencies: [
                "PocketKit",
                .product(name: "MLX", package: "mlx-swift"),
                .product(name: "MLXLLM", package: "mlx-swift-lm"),
                .product(name: "MLXLMCommon", package: "mlx-swift-lm"),
                .product(name: "MLXEmbedders", package: "mlx-swift-lm"),
                .product(name: "Tokenizers", package: "swift-transformers"),
            ],
            path: "swift/Sources/PocketMLX"
        ),
        .testTarget(name: "PocketKitTests", dependencies: ["PocketKit"], path: "swift/Tests/PocketKitTests"),
        .testTarget(name: "PocketUITests", dependencies: ["PocketUI"], path: "swift/Tests/PocketUITests"),
        .testTarget(name: "PocketMLXTests", dependencies: ["PocketMLX", "PocketKit"], path: "swift/Tests/PocketMLXTests"),
    ]
)
