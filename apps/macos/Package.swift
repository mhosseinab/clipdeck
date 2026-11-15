// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "ClipDeck",
    platforms: [.macOS(.v13)],
    targets: [
        .target(name: "ClipDeckCore"),
        .executableTarget(name: "ClipDeckApp", dependencies: ["ClipDeckCore"]),
        .testTarget(name: "ClipDeckCoreTests", dependencies: ["ClipDeckCore"]),
    ]
)
