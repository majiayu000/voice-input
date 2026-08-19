// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "VoiceInputApp",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "VoiceInputApp", targets: ["VoiceInputApp"])
    ],
    targets: [
        .executableTarget(
            name: "VoiceInputApp",
            path: "Sources"
        ),
        .testTarget(
            name: "VoiceInputAppTests",
            dependencies: ["VoiceInputApp"],
            path: "Tests"
        )
    ],
    swiftLanguageModes: [.v5]
)
