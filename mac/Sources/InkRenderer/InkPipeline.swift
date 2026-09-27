// The ink's one Metal pipeline, shared by every surface that draws it (the Drop, the rail,
// Today's ink zone) and by offscreen renders.
//
// The shader ships as source: Resources/ink.msl, generated from shaders/ink.wgsl by
// core/crates/ink-shader, compiled here with makeLibrary(source:). Building the app needs no Metal
// toolchain, and the shells share one WGSL source. The compile runs once, off the main thread
// (InkPipelineLoader).
import Foundation
import Metal
import Synchronization

/// Why the ink cannot draw. The app then shows plain paper where the ink would be.
public enum InkRendererError: Error, Equatable, CustomStringConvertible {
    /// This Mac has no Metal device.
    case noDevice
    /// The bundled shader was not found (a broken build).
    case shaderMissing
    /// The shader did not compile or link; the message is Metal's.
    case shaderFailed(String)
    /// Metal refused to make a resource.
    case resource(String)

    public var description: String {
        switch self {
        case .noDevice: "no Metal device"
        case .shaderMissing: "the ink shader (ink.msl) is missing from the app"
        case .shaderFailed(let message): "the ink shader did not compile: \(message)"
        case .resource(let what): "Metal could not make the ink's \(what)"
        }
    }
}

/// The compiled shader, its sampler, and a queue. Thread-safe: Metal's device, queue, pipeline and
/// sampler objects may be used from any thread, and nothing here changes after `init`.
public final class InkPipeline: @unchecked Sendable {
    // @unchecked: every stored property is an immutable reference to a Metal object that Metal
    // documents as thread-safe (device, command queue, render pipeline state, sampler state), or
    // a texture only ever read after it was filled in `init`.

    public let device: any MTLDevice
    let queue: any MTLCommandQueue
    let pipeline: any MTLRenderPipelineState
    let sampler: any MTLSamplerState
    /// Bound while there is no wordmark: the shader always declares the texture.
    let noMark: any MTLTexture

    /// The pixel format every ink target uses: the drawable's, and the offscreen render's.
    public static let pixelFormat = MTLPixelFormat.bgra8Unorm

    init(source: String? = nil) throws {
        guard let device = MTLCreateSystemDefaultDevice() else { throw InkRendererError.noDevice }
        guard let queue = device.makeCommandQueue() else { throw InkRendererError.resource("command queue") }
        guard let source = source ?? InkShaderSource.load() else { throw InkRendererError.shaderMissing }
        let library: any MTLLibrary
        do {
            library = try device.makeLibrary(source: source, options: MTLCompileOptions())
        } catch {
            throw InkRendererError.shaderFailed(error.localizedDescription)
        }
        guard let vertex = library.makeFunction(name: "vs_main"),
            let fragment = library.makeFunction(name: "fs_main")
        else { throw InkRendererError.shaderFailed("no vs_main or fs_main") }
        let descriptor = MTLRenderPipelineDescriptor()
        descriptor.label = "ink"
        descriptor.vertexFunction = vertex
        descriptor.fragmentFunction = fragment
        descriptor.colorAttachments[0].pixelFormat = Self.pixelFormat
        do {
            pipeline = try device.makeRenderPipelineState(descriptor: descriptor)
        } catch {
            throw InkRendererError.shaderFailed(error.localizedDescription)
        }
        let samplerDescriptor = MTLSamplerDescriptor()
        samplerDescriptor.minFilter = .linear
        samplerDescriptor.magFilter = .linear
        samplerDescriptor.mipFilter = .notMipmapped
        samplerDescriptor.sAddressMode = .clampToEdge
        samplerDescriptor.tAddressMode = .clampToEdge
        guard let sampler = device.makeSamplerState(descriptor: samplerDescriptor) else {
            throw InkRendererError.resource("sampler")
        }
        self.device = device
        self.queue = queue
        self.sampler = sampler
        noMark = try Self.markTexture(device: device, coverage: [0], width: 1, height: 1)
    }

    /// A wordmark texture: one byte of coverage per pixel, which the shader reads as alpha.
    public func markTexture(_ mark: Wordmark) throws -> any MTLTexture {
        try Self.markTexture(device: device, coverage: mark.coverage, width: mark.width, height: mark.height)
    }

    private static func markTexture(device: any MTLDevice, coverage: [UInt8], width: Int, height: Int) throws
        -> any MTLTexture
    {
        guard width > 0, height > 0, coverage.count == width * height else {
            throw InkRendererError.resource("wordmark texture (\(width)x\(height))")
        }
        let descriptor = MTLTextureDescriptor.texture2DDescriptor(
            pixelFormat: .a8Unorm, width: width, height: height, mipmapped: false)
        descriptor.usage = [.shaderRead]
        descriptor.storageMode = .shared
        guard let texture = device.makeTexture(descriptor: descriptor) else {
            throw InkRendererError.resource("wordmark texture (\(width)x\(height))")
        }
        coverage.withUnsafeBytes { bytes in
            // Non-nil: coverage holds width * height > 0 bytes (checked above).
            if let base = bytes.baseAddress {
                texture.replace(region: MTLRegionMake2D(0, 0, width, height), mipmapLevel: 0,
                                withBytes: base, bytesPerRow: width)
            }
        }
        return texture
    }

    /// Encodes one full-canvas draw into `target`. Allocation-free apart from Metal's own encoder.
    public func encode(
        into target: any MTLTexture, commandBuffer: any MTLCommandBuffer, uniforms: InkUniforms,
        mark: (any MTLTexture)?
    ) {
        let pass = MTLRenderPassDescriptor()
        pass.colorAttachments[0].texture = target
        // Every pixel is written: nothing to load.
        pass.colorAttachments[0].loadAction = .dontCare
        pass.colorAttachments[0].storeAction = .store
        guard let encoder = commandBuffer.makeRenderCommandEncoder(descriptor: pass) else { return }
        encoder.label = "ink"
        encoder.setRenderPipelineState(pipeline)
        withUnsafeBytes(of: uniforms) { bytes in
            // Non-nil: InkUniforms is 144 bytes.
            if let base = bytes.baseAddress {
                encoder.setFragmentBytes(base, length: bytes.count, index: 0)
            }
        }
        encoder.setFragmentTexture(mark ?? noMark, index: 0)
        encoder.setFragmentSamplerState(sampler, index: 0)
        encoder.drawPrimitives(type: .triangleStrip, vertexStart: 0, vertexCount: 4)
        encoder.endEncoding()
    }
}

/// Compiles the ink's pipeline once, on a background queue, and hands it to everything that waits
/// for it. The app starts the compile in main(), next to the core's start, so no ink view ever
/// compiles on the main thread: a view made before the compile finishes shows plain paper and
/// starts drawing when the pipeline arrives. A failure is kept: a Mac without Metal, or a build
/// without the shader, does not retry.
public final class InkPipelineLoader: Sendable {
    public typealias Outcome = Result<InkPipeline, InkRendererError>

    private enum Phase {
        case idle
        case compiling
        case done(Outcome)
    }

    private struct State {
        var phase = Phase.idle
        var duration: Duration?
        var waiters: [@Sendable (Outcome) -> Void] = []
    }

    private let make: @Sendable () throws -> InkPipeline
    private let state = Mutex(State())

    /// The process's loader: the bundled shader.
    public static let shared = InkPipelineLoader { try InkPipeline() }

    /// A loader whose compile is `make` (tests hold it back or make it fail).
    public init(make: @escaping @Sendable () throws -> InkPipeline) {
        self.make = make
    }

    /// Starts the compile on a background queue, the first time only. Any thread; never waits.
    public func warm() {
        let start = state.withLock { state -> Bool in
            guard case .idle = state.phase else { return false }
            state.phase = .compiling
            return true
        }
        guard start else { return }
        DispatchQueue.global(qos: .userInitiated).async { [self] in
            let clock = ContinuousClock()
            let began = clock.now
            let outcome: Outcome
            do {
                outcome = .success(try make())
            } catch let error as InkRendererError {
                outcome = .failure(error)
            } catch {
                outcome = .failure(.shaderFailed(String(describing: error)))
            }
            let took = clock.now - began
            let waiters = state.withLock { state -> [@Sendable (Outcome) -> Void] in
                state.phase = .done(outcome)
                state.duration = took
                defer { state.waiters = [] }
                return state.waiters
            }
            for waiter in waiters { waiter(outcome) }
        }
    }

    /// The outcome once the compile has finished, else nil. Never waits.
    public var outcome: Outcome? {
        state.withLock { state in
            if case .done(let outcome) = state.phase { return outcome }
            return nil
        }
    }

    /// How long the compile took, once it has finished.
    public var compileDuration: Duration? {
        state.withLock { $0.duration }
    }

    /// Calls `body` on the main thread with the outcome: on a later turn of the main queue, even
    /// when the compile has already finished. Starts the compile if nothing has.
    public func whenReady(_ body: @escaping @MainActor @Sendable (Outcome) -> Void) {
        warm()
        subscribe { outcome in
            DispatchQueue.main.async { MainActor.assumeIsolated { body(outcome) } }
        }
    }

    /// Waits for the outcome, starting the compile if nothing has. For tests and offline renders:
    /// the app's main thread uses `whenReady` instead.
    public func wait() -> Outcome {
        warm()
        let done = DispatchSemaphore(value: 0)
        let result = Mutex<Outcome?>(nil)
        subscribe { outcome in
            result.withLock { $0 = outcome }
            done.signal()
        }
        done.wait()
        // Set before the signal above.
        return result.withLock { $0 } ?? .failure(.resource("pipeline"))
    }

    /// Runs `waiter` with the outcome: now if there is one, else on the compiling thread when it
    /// finishes.
    private func subscribe(_ waiter: @escaping @Sendable (Outcome) -> Void) {
        let ready = state.withLock { state -> Outcome? in
            if case .done(let outcome) = state.phase { return outcome }
            state.waiters.append(waiter)
            return nil
        }
        if let ready { waiter(ready) }
    }
}

/// Finds the bundled shader source.
enum InkShaderSource {
    /// The resource bundle's name: SwiftPM names it <package>_<target>.
    static let bundleName = "Inkwell_InkRenderer.bundle"

    /// Resources/ink.msl, from the first place a build puts the bundle: an app's Resources (the
    /// build script copies it there), next to a command-line binary, or next to the test bundle.
    /// Nil when none has it: never a crash (SwiftPM's own Bundle.module traps instead).
    static func load() -> String? {
        final class Anchor {}
        let places = [
            Bundle.main.resourceURL,
            Bundle.main.bundleURL,
            Bundle(for: Anchor.self).resourceURL,
            Bundle(for: Anchor.self).bundleURL.deletingLastPathComponent(),
        ]
        for place in places.compactMap({ $0 }) {
            let bundle = place.appendingPathComponent(bundleName)
            guard let url = Bundle(url: bundle)?.url(forResource: "ink", withExtension: "msl"),
                let source = try? String(contentsOf: url, encoding: .utf8)
            else { continue }
            return source
        }
        return nil
    }
}
