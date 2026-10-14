// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "PocketCore",
    platforms: [.iOS(.v18), .macOS(.v15)],
    products: [
        .library(name: "PocketKit", targets: ["PocketKit"]),
        .library(name: "PocketUI", targets: ["PocketUI"]),
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
            ]
        ),
        .target(
            name: "PocketUI",
            path: "swift/Sources/PocketUI",
            resources: [.process("Resources/Colors.xcassets"), .process("Resources/MenuBar.xcassets")]
        ),
        .testTarget(name: "PocketKitTests", dependencies: ["PocketKit"], path: "swift/Tests/PocketKitTests"),
        .testTarget(name: "PocketUITests", dependencies: ["PocketUI"], path: "swift/Tests/PocketUITests"),
    ]
)
