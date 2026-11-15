// swift-tools-version: 6.0
import PackageDescription

// The generated UniFFI bindings live in their own module so they compile without the app's default main-actor
// isolation. Sources and the xcframework come from scripts/build-core.sh.
let package = Package(
    name: "NaviaCoreKit",
    platforms: [.macOS("26.0")],
    products: [.library(name: "NaviaCoreFFI", targets: ["NaviaCoreFFI"])],
    targets: [
        .binaryTarget(name: "navia_ffiFFI", path: "navia_ffiFFI.xcframework"),
        .target(
            name: "NaviaCoreFFI",
            dependencies: ["navia_ffiFFI"],
            path: "Sources/NaviaCoreFFI",
            swiftSettings: [.swiftLanguageMode(.v5)],
            linkerSettings: [
                .linkedFramework("CoreAudio"), .linkedFramework("AudioToolbox"), .linkedFramework("AudioUnit"),
                .linkedFramework("CoreFoundation"), .linkedFramework("Security"), .linkedFramework("SystemConfiguration"),
                .linkedLibrary("c++"),
            ]
        ),
    ]
)
