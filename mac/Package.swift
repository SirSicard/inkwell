// swift-tools-version: 6.2
// The Mac app. No .xcodeproj: SwiftPM builds everything, scripts/build-mac.sh bundles and signs it,
// and .github/workflows/mac-release.yml notarizes and releases it (docs/RELEASING.md).
//
// | Target       | Holds                                                                    |
// |--------------|--------------------------------------------------------------------------|
// | InkCore      | the Rust core as an XCFramework (scripts/build-core.sh), C ABI inkwell.h |
// | InkBridge    | Swift over the C ABI: the session, engine registration, generated events |
// | AppleEngines | FluidAudio and Foundation Models, registered into the core (S2.2)        |
// | InkRenderer  | the Metal ink (S2.4)                                                     |
// | Inkwell      | the app: SwiftUI and AppKit (S2.3)                                       |
// | Sparkle      | in-app updates: Sparkle's prebuilt XCFramework, pinned by checksum       |
//
// Each target links the system frameworks it uses; InkBridge links what the core's static library
// needs (`cargo rustc -p ink-ffi --crate-type staticlib -- --print native-static-libs`).
import PackageDescription

let package = Package(
    name: "Inkwell",
    platforms: [.macOS(.v26)],
    products: [
        .executable(name: "Inkwell", targets: ["Inkwell"]),
    ],
    dependencies: [
        // Parakeet on the Neural Engine, for live partials (AppleEngines). Apache-2.0; pinned to
        // the version the engine choice was measured with.
        .package(url: "https://github.com/FluidInference/FluidAudio.git", exact: "0.15.5"),
    ],
    targets: [
        .binaryTarget(name: "InkCore", path: "build/InkCore.xcframework"),
        // In-app updates: Sparkle 2.10.0 (MIT, with BSD-2-Clause and Zlib parts; THIRD_PARTY.md).
        // Sparkle's own release archive, pinned here by URL and SHA-256 rather than through its
        // package, so the one download is the vetted archive and no repository is cloned. SwiftPM
        // refuses an archive whose hash differs; scripts/licence-audit-swift.sh refuses a URL or
        // checksum that differs from its vetted entry.
        .binaryTarget(
            name: "Sparkle",
            url: "https://github.com/sparkle-project/Sparkle/releases/download/2.10.0/Sparkle-for-Swift-Package-Manager.zip",
            checksum: "17e28312b8e18ab7cdbbe09a6fb28cc55a5479ec6c371dbc07cdecd2a14fd959"
        ),
        .target(
            name: "InkBridge",
            dependencies: ["InkCore"],
            linkerSettings: [
                .linkedFramework("AppKit"),
                .linkedFramework("ApplicationServices"),
                .linkedFramework("AVFoundation"),
                .linkedFramework("Carbon"),
                .linkedFramework("CoreAudio"),
                .linkedFramework("CoreFoundation"),
                .linkedFramework("CoreGraphics"),
                .linkedFramework("Foundation"),
                .linkedFramework("Security"),
                .linkedLibrary("iconv"),
                .linkedLibrary("objc"),
            ]
        ),
        .target(
            name: "AppleEngines",
            dependencies: ["InkBridge", .product(name: "FluidAudio", package: "FluidAudio")],
            linkerSettings: [
                .linkedFramework("Accelerate"),
                .linkedFramework("AVFoundation"),
                .linkedFramework("CoreML"),
                .linkedFramework("FoundationModels"),
            ]
        ),
        .target(
            name: "InkRenderer",
            // The shader as MSL source, generated from shaders/ink.wgsl (core/crates/ink-shader) and
            // compiled at run time. Copied as is: a .msl file is never built by a Metal toolchain.
            resources: [.copy("Resources/ink.msl")],
            linkerSettings: [
                .linkedFramework("CoreText"),
                .linkedFramework("Metal"),
                .linkedFramework("MetalKit"),
                .linkedFramework("QuartzCore"),
            ]
        ),
        .executableTarget(
            name: "Inkwell",
            dependencies: ["InkBridge", "AppleEngines", "InkRenderer", "Sparkle"],
            linkerSettings: [
                .linkedFramework("AppKit"),
                .linkedFramework("ServiceManagement"),
                .linkedFramework("SwiftUI"),
            ]
        ),
        .testTarget(name: "InkBridgeTests", dependencies: ["InkBridge"]),
        .testTarget(name: "AppleEnginesTests", dependencies: ["AppleEngines", "InkBridge"]),
        .testTarget(name: "InkwellTests", dependencies: ["Inkwell"]),
        .testTarget(name: "InkRendererTests", dependencies: ["InkRenderer"]),
    ],
    swiftLanguageModes: [.v6]
)
