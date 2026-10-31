// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "VayoraKit",
    defaultLocalization: "en",
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [.library(name: "VayoraKit", targets: ["VayoraKit"])],
    targets: [
        .target(name: "VayoraKit"),
        .testTarget(name: "VayoraKitTests", dependencies: ["VayoraKit"]),
    ]
)
