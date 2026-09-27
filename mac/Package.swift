// swift-tools-version: 6.2
// The Mac app. No .xcodeproj: SwiftPM builds everything (the release bundle comes in S2.9b).
//
// | Target       | Holds                                                                    |
// |--------------|--------------------------------------------------------------------------|
// | InkCore      | the Rust core as an XCFramework (scripts/build-core.sh), C ABI inkwell.h |
// | InkBridge    | Swift over the C ABI: the session, engine registration, generated events |
// | AppleEngines | FluidAudio and Foundation Models, registered into the core (S2.2)        |
// | InkRenderer  | the Metal ink (S2.4)                                                     |
// | Inkwell      | the app: SwiftUI and AppKit (S2.3)                                       |
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
    targets: [
        .binaryTarget(name: "InkCore", path: "build/InkCore.xcframework"),
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
            dependencies: ["InkBridge"],
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
            dependencies: ["InkBridge", "AppleEngines", "InkRenderer"],
            linkerSettings: [
                .linkedFramework("AppKit"),
                .linkedFramework("ServiceManagement"),
                .linkedFramework("SwiftUI"),
            ]
        ),
        .testTarget(name: "InkBridgeTests", dependencies: ["InkBridge"]),
        .testTarget(name: "InkwellTests", dependencies: ["Inkwell"]),
        .testTarget(name: "InkRendererTests", dependencies: ["InkRenderer"]),
    ],
    swiftLanguageModes: [.v6]
)
