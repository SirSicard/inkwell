// One frame of the orb, offscreen, at a fixed time: what the tests check. No window and no display
// link, so it also runs with the screen locked and on a Mac with no display.
//
// The recipe: a zeroed state at time `t`, one step on the snap path (the weights at the state's
// targets), the prototype's synthetic voice. `voice: .levels` replaces the voice with fixed
// levels.
import CoreGraphics
import Foundation
import ImageIO
import Metal
import UniformTypeIdentifiers

/// A rendered frame.
public struct InkImage: Sendable {
    public var width: Int
    public var height: Int
    /// 8-bit RGBA, premultiplied, row 0 at the top; alpha 0 where there is no orb.
    public var rgba: [UInt8]
    /// The uniforms it was drawn with.
    public var uniforms: InkUniforms

    /// The pixel at (x, y) from the top left, as (r, g, b), premultiplied.
    public func pixel(_ x: Int, _ y: Int) -> (r: UInt8, g: UInt8, b: UInt8) {
        let i = (y * width + x) * 4
        return (rgba[i], rgba[i + 1], rgba[i + 2])
    }

    /// The pixel's alpha, 0...255.
    public func alpha(_ x: Int, _ y: Int) -> UInt8 {
        rgba[(y * width + x) * 4 + 3]
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
                space: space, bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue),
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
    /// Draws `state` at time `t` into a `width` x `height` pixel canvas, in `palette`'s colours, the
    /// orb at `placement`. `motion` false draws the still frame (the shader's time stopped);
    /// `blotDepth` is InkSimulation's.
    public static func render(
        _ state: InkState, t: Double, width: Int, height: Int, palette: OrbPalette = .neutral,
        placement: OrbPlacement = .centred, voice: InkVoice = .synthetic, motion: Bool = true,
        blotDepth: Double = 1, pipeline: InkPipeline
    ) throws -> InkImage {
        guard width >= 2, height >= 2 else { throw InkRendererError.resource("canvas \(width)x\(height)") }
        var simulation = InkSimulation(random: .seeded(1))
        simulation.state = state
        simulation.blotDepth = blotDepth
        simulation.canvasWidth = Double(width)
        simulation.canvasHeight = Double(height)
        simulation.applyFixed(t: t, voice: voice)
        let uniforms = simulation.uniforms(palette: palette, placement: placement, motion: motion)

        let descriptor = MTLTextureDescriptor.texture2DDescriptor(
            pixelFormat: InkPipeline.pixelFormat, width: width, height: height, mipmapped: false)
        descriptor.usage = [.renderTarget]
        descriptor.storageMode = .shared
        guard let target = pipeline.device.makeTexture(descriptor: descriptor),
            let commandBuffer = pipeline.queue.makeCommandBuffer()
        else { throw InkRendererError.resource("offscreen target \(width)x\(height)") }
        pipeline.encode(into: target, commandBuffer: commandBuffer, uniforms: uniforms)
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
        // BGRA to RGBA; alpha as drawn.
        var rgba = bgra
        for i in stride(from: 0, to: rgba.count, by: 4) {
            rgba[i] = bgra[i + 2]
            rgba[i + 2] = bgra[i]
        }
        return InkImage(width: width, height: height, rgba: rgba, uniforms: uniforms)
    }
}
