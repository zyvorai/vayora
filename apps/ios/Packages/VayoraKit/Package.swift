// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "VayoraKit",
    defaultLocalization: "en",
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [
        .library(name: "VayoraKit", targets: ["VayoraKit"]),
        .library(name: "VayoraUI", targets: ["VayoraUI"]),
    ],
    targets: [
        .target(name: "VayoraKit"),
        .target(name: "VayoraUI", dependencies: ["VayoraKit"]),
        // Renders screens to PNG on the Mac so the design can be reviewed without a simulator.
        .executableTarget(name: "VayoraGallery", dependencies: ["VayoraUI", "VayoraKit"]),
        .testTarget(name: "VayoraKitTests", dependencies: ["VayoraKit"]),
        .testTarget(name: "VayoraUITests", dependencies: ["VayoraUI"]),
    ]
)
