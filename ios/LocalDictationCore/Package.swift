// swift-tools-version: 5.9
import PackageDescription
let package = Package(
    name: "LocalDictationCore",
    platforms: [.macOS(.v13), .iOS(.v18)],
    products: [.library(name: "LocalDictationCore", targets: ["LocalDictationCore"])],
    targets: [.target(name: "LocalDictationCore"),
              .testTarget(name: "LocalDictationCoreTests", dependencies: ["LocalDictationCore"])]
)
