// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "PocketCore",
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [
        .library(name: "PocketKit", targets: ["PocketKit"]),
    ],
    targets: [
        .binaryTarget(name: "pocket_ffiFFI", path: "build/pocket_ffiFFI.xcframework"),
        .target(name: "PocketKit", dependencies: ["pocket_ffiFFI"], path: "swift/Sources/PocketKit"),
        .testTarget(name: "PocketKitTests", dependencies: ["PocketKit"], path: "swift/Tests/PocketKitTests"),
    ]
)
