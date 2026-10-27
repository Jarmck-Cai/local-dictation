// swift-tools-version: 5.9
import PackageDescription
let package = Package(
    name: "LocalDictationCore",
    platforms: [.macOS(.v13), .iOS("18.0")],
    products: [.library(name: "LocalDictationCore", targets: ["LocalDictationCore"])],
    targets: [.target(name: "LocalDictationCore"),
              .testTarget(name: "LocalDictationCoreTests", dependencies: ["LocalDictationCore"])]
)
