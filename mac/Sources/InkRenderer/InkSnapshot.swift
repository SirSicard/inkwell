// One frame of the ink, offscreen, at a fixed time: what the tests check and what is compared
// with the prototype's WebGL renders. No window and no display link, so it also runs with the
// screen locked and on a Mac with no display.
//
// The recipe is the reference renders' (the fixed path): a zeroed state at time `t`, droplets
// off, one step on the snap path, the prototype's synthetic voice. `voice: .levels` replaces the
// voice with fixed levels.
import CoreGraphics
import Foundation
import ImageIO
import Metal
import UniformTypeIdentifiers

/// A rendered frame.
public struct InkImage: Sendable {
    public var width: Int
    public var height: Int
    /// 8-bit RGBA, row 0 at the top, alpha 255.
    public var rgba: [UInt8]
    /// The uniforms it was drawn with.
    public var uniforms: InkUniforms
    /// The wordmark, when drawn.
    public var wordmark: Wordmark?

    /// The pixel at (x, y) from the top left, as (r, g, b).
    public func pixel(_ x: Int, _ y: Int) -> (r: UInt8, g: UInt8, b: UInt8) {
        let i = (y * width + x) * 4
        return (rgba[i], rgba[i + 1], rgba[i + 2])
    }

    /// Rec. 601 luminance, 0...255.
    public func luminance(_ x: Int, _ y: Int) -> Double {
        let p = pixel(x, y)
        return 0.299 * Double(p.r) + 0.587 * Double(p.g) + 0.114 * Double(p.b)
    }

    /// Writes it as a PNG (sRGB).
    public func writePNG(to url: URL) throws {
        guard let space = CGColorSpace(name: CGColorSpace.sRGB),
            let provider = CGDataProvider(data: Data(rgba) as CFData),
            let image = CGImage(
                width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: width * 4,
                space: space, bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.noneSkipLast.rawValue),
                provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent),
            let destination = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil)
        else { throw InkRendererError.resource("PNG at \(url.lastPathComponent)") }
        CGImageDestinationAddImage(destination, image, nil)
        guard CGImageDestinationFinalize(destination) else {
            throw InkRendererError.resource("PNG at \(url.lastPathComponent)")
        }
    }
}

/// Offscreen frames.
public enum InkSnapshot {
    /// Draws `state` at time `t` into a `width` x `height` pixel canvas. `pointWidth` is the
    /// canvas width in points, which sizes the wordmark (default: one pixel per point, as the
    /// reference renders are).
    public static func render(
        _ state: InkState, t: Double, width: Int, height: Int, pointWidth: Double? = nil,
        wordmark: Bool = true, font: WordmarkFont = .system, voice: InkVoice = .synthetic, cy: Double = 0.5,
        pipeline: InkPipeline
    ) throws -> InkImage {
        guard width >= 2, height >= 2 else { throw InkRendererError.resource("canvas \(width)x\(height)") }
        var simulation = InkSimulation(random: .seeded(1))
        simulation.state = state
        simulation.cy = cy
        simulation.canvasWidth = Double(width)
        simulation.canvasHeight = Double(height)
        simulation.applyFixed(t: t, voice: voice)

        var mark: Wordmark?
        var markTexture: (any MTLTexture)?
        if wordmark {
            let raster = Wordmark.rasterize(width: width, height: height, pointWidth: pointWidth ?? Double(width),
                                            font: font)
            markTexture = try pipeline.markTexture(raster)
            mark = raster
        }
        let uniforms = simulation.uniforms(hasMark: markTexture != nil)

        let descriptor = MTLTextureDescriptor.texture2DDescriptor(
            pixelFormat: InkPipeline.pixelFormat, width: width, height: height, mipmapped: false)
        descriptor.usage = [.renderTarget]
        descriptor.storageMode = .shared
        guard let target = pipeline.device.makeTexture(descriptor: descriptor),
            let commandBuffer = pipeline.queue.makeCommandBuffer()
        else { throw InkRendererError.resource("offscreen target \(width)x\(height)") }
        pipeline.encode(into: target, commandBuffer: commandBuffer, uniforms: uniforms, mark: markTexture)
        commandBuffer.commit()
        commandBuffer.waitUntilCompleted()
        if let error = commandBuffer.error {
            throw InkRendererError.resource("frame (\(error.localizedDescription))")
        }

        var bgra = [UInt8](repeating: 0, count: width * height * 4)
        bgra.withUnsafeMutableBytes { bytes in
            if let base = bytes.baseAddress {
                target.getBytes(base, bytesPerRow: width * 4, from: MTLRegionMake2D(0, 0, width, height), mipmapLevel: 0)
            }
        }
        // BGRA to RGBA.
        var rgba = bgra
        for i in stride(from: 0, to: rgba.count, by: 4) {
            rgba[i] = bgra[i + 2]
            rgba[i + 2] = bgra[i]
            rgba[i + 3] = 255
        }
        return InkImage(width: width, height: height, rgba: rgba, uniforms: uniforms, wordmark: mark)
    }
}
