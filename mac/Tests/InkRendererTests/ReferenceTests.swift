// Checks that need something CI does not have, each skipped unless its variable is set:
//
//   INK_REFERENCE_DIR  a directory of the design prototype's own WebGL renders, webgl-<state>.png
//                      (360 x 720, t = 12, wordmark on, drawn in Helvetica Neue Bold). Every state
//                      is rendered twice, with the app's face (SF Pro Black) and with the
//                      reference's, and each must match: no pixel off by more than 8/255 outside
//                      the wordmark's box, SSIM over the ink outside that box at least 0.95, and
//                      SSIM over the whole frame at least 0.9. In the reference's face the ink
//                      score counting the letters as ink (as the first comparison did) must also
//                      reach 0.95; in the app's face the letters differ by design.
//   INK_RENDER_OUT     where to write this renderer's frames (metal-<state>[-<face>].png), to
//                      compare them with another tool.
//   INK_GPU_TIMING=1   GPU time per frame at each live canvas size, paced at 60 fps: under 2 ms.
//
// The scores follow Wang, Bovik, Sheikh and Simoncelli (2004) and their reference implementation:
// luminance (Rec. 601), an 11 x 11 Gaussian window with sigma 1.5, K1 = 0.01, K2 = 0.03, statistics
// over the valid region only, the mean of the map. testTheScorerOnKnownAnswers checks the scorer
// before any score is trusted.
import CoreGraphics
import Foundation
import ImageIO
import Metal
import XCTest

@testable import InkRenderer

final class ReferenceTests: XCTestCase {
    private let environment = ProcessInfo.processInfo.environment

    func testTheScorerOnKnownAnswers() {
        var rng = InkRandom.seeded(7)
        let w = 40, h = 30
        let image = (0..<(w * h)).map { _ in 255 * rng.next() }
        XCTAssertEqual(SSIM.map(image, image, width: w, height: h).mean, 1, accuracy: 1e-12, "identical")
        let a = [Double](repeating: 100, count: w * h), b = [Double](repeating: 120, count: w * h)
        let c1: Double = (0.01 * 255) * (0.01 * 255)
        let flat: Double = (2.0 * 100 * 120 + c1) / (100.0 * 100 + 120 * 120 + c1)
        XCTAssertEqual(SSIM.map(a, b, width: w, height: h).mean, flat, accuracy: 1e-12,
                       "flat images: only the luminance term")
        let inverted = image.map { 255 - $0 }
        XCTAssertLessThan(SSIM.map(image, inverted, width: w, height: h).mean, 0.2)
        let ab = SSIM.map(image, inverted, width: w, height: h).mean
        let ba = SSIM.map(inverted, image, width: w, height: h).mean
        XCTAssertEqual(ab, ba, accuracy: 1e-12, "symmetric")
        XCTAssertEqual(SSIM.map(image, image, width: w, height: h).values.count, (w - 10) * (h - 10), "valid region")
    }

    func testEveryStateMatchesThePrototypesWebGLRender() throws {
        guard let directory = environment["INK_REFERENCE_DIR"] else {
            throw XCTSkip("INK_REFERENCE_DIR is not set")
        }
        try XCTSkipUnless(InkRenderer.isSupported, "no Metal device")
        let pipeline = try InkPipelineLoader.shared.wait().get()
        let out = environment["INK_RENDER_OUT"].map { URL(fileURLWithPath: $0, isDirectory: true) }
        if let out { try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true) }
        // The two faces' boxes together, grown by 2 px for antialiasing: outside it the frames must
        // agree to 8/255 whichever face drew the letters.
        let system = Wordmark.rasterize(width: 360, height: 720, pointWidth: 360, font: .system)
        let reference = Wordmark.rasterize(width: 360, height: 720, pointWidth: 360, font: .named("HelveticaNeue-Bold"))
        let box = system.box.union(reference.box).insetBy(dx: -2, dy: -2)
        print("wordmark box \(box) (system \(system.fontName), reference \(reference.fontName))")

        for state in InkState.allCases {
            let url = URL(fileURLWithPath: directory).appendingPathComponent("webgl-\(state.rawValue).png")
            let webgl = try RGBImage(png: url)
            XCTAssertEqual(webgl.width, 360)
            XCTAssertEqual(webgl.height, 720)
            for (face, font) in [("system", WordmarkFont.system), ("reference-face", .named("HelveticaNeue-Bold"))] {
                let ours = try InkSnapshot.render(state, t: 12, width: 360, height: 720, font: font, pipeline: pipeline)
                if let out {
                    let suffix = face == "system" ? "" : "-\(face)"
                    try ours.writePNG(to: out.appendingPathComponent("metal-\(state.rawValue)\(suffix).png"))
                }
                let score = Comparison(ours, webgl, box: box)
                print("\(state.rawValue) [\(face)] \(score)")
                XCTAssertEqual(score.offOutsideBox, 0, "\(state) [\(face)]: pixels off by more than 8/255 outside the wordmark")
                XCTAssertGreaterThanOrEqual(score.inkSSIMOutsideBox, 0.95, "\(state) [\(face)]: SSIM over the ink")
                XCTAssertGreaterThanOrEqual(score.wholeSSIM, 0.9, "\(state) [\(face)]: SSIM over the frame")
                if face != "system" {
                    XCTAssertGreaterThanOrEqual(score.inkSSIM, 0.95, "\(state) [\(face)]: SSIM over the ink and letters")
                }
            }
        }
    }

    /// GPU time per frame at the sizes the app draws, paced like the display link: the Drop's
    /// zone (84 pt at 2x), the rail (56 x 700 pt at 2x), Today's zone (300 x 700 pt, capped at
    /// 1.25x), and the 360 x 720 pt panel the shell budget was first measured with (1.25x).
    func testGPUTimePerFrameAt60FPS() throws {
        guard environment["INK_GPU_TIMING"] == "1" else { throw XCTSkip("INK_GPU_TIMING is not 1") }
        try XCTSkipUnless(InkRenderer.isSupported, "no Metal device")
        let pipeline = try InkPipelineLoader.shared.wait().get()
        for (name, w, h, pointWidth, mark) in [
            ("drop", 168, 168, 84.0, false), ("rail", 112, 1400, 56.0, false),
            ("today", 375, 875, 300.0, true), ("panel", 450, 900, 360.0, true),
        ] {
            let descriptor = MTLTextureDescriptor.texture2DDescriptor(
                pixelFormat: InkPipeline.pixelFormat, width: w, height: h, mipmapped: false)
            descriptor.usage = [.renderTarget]
            descriptor.storageMode = .private
            let target = try XCTUnwrap(pipeline.device.makeTexture(descriptor: descriptor))
            let markTexture = mark
                ? try pipeline.markTexture(Wordmark.rasterize(width: w, height: h, pointWidth: pointWidth)) : nil
            var sim = InkSimulation(random: .seeded(1))
            sim.state = .meeting
            sim.canvasWidth = Double(w)
            sim.canvasHeight = Double(h)
            sim.t = 12
            var times: [Double] = []
            for frame in 0..<330 {
                sim.step(1.0 / 60, snap: false, voice: .synthetic)
                let commandBuffer = try XCTUnwrap(pipeline.queue.makeCommandBuffer())
                pipeline.encode(into: target, commandBuffer: commandBuffer,
                                uniforms: sim.uniforms(hasMark: markTexture != nil), mark: markTexture)
                commandBuffer.commit()
                commandBuffer.waitUntilCompleted()
                // The first 30 frames warm the pipeline and the clocks.
                if frame >= 30 { times.append((commandBuffer.gpuEndTime - commandBuffer.gpuStartTime) * 1000) }
                Thread.sleep(forTimeInterval: 1.0 / 60)
            }
            times.sort()
            let p50 = times[times.count / 2], p95 = times[Int(Double(times.count - 1) * 0.95)]
            print(String(format: "gpu %@ %dx%d: p50 %.3f ms, p95 %.3f ms, max %.3f ms over %d frames",
                         name, w, h, p50, p95, times.last ?? 0, times.count))
            XCTAssertLessThan(p95, 2, "\(name): GPU time per frame at 60 fps")
        }
    }
}

/// An 8-bit image read without colour conversion: the stored bytes, as the reference script reads
/// them.
struct RGBImage {
    var width: Int
    var height: Int
    var rgb: [UInt8]

    init(png url: URL) throws {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
            let image = CGImageSourceCreateImageAtIndex(source, 0, nil),
            let data = image.dataProvider?.data as Data?
        else { throw InkRendererError.resource("reference image \(url.lastPathComponent)") }
        let bytesPerPixel = image.bitsPerPixel / 8
        guard image.bitsPerComponent == 8, bytesPerPixel == 3 || bytesPerPixel == 4,
            image.byteOrderInfo == .orderDefault || image.byteOrderInfo == .order32Big,
            [.none, .last, .noneSkipLast, .premultipliedLast].contains(image.alphaInfo)
        else { throw InkRendererError.resource("reference image format \(url.lastPathComponent)") }
        let w = image.width, h = image.height, stride = image.bytesPerRow
        var rgb = [UInt8](repeating: 0, count: w * h * 3)
        data.withUnsafeBytes { raw in
            for y in 0..<h {
                for x in 0..<w {
                    let s = y * stride + x * bytesPerPixel, d = (y * w + x) * 3
                    rgb[d] = raw[s]
                    rgb[d + 1] = raw[s + 1]
                    rgb[d + 2] = raw[s + 2]
                }
            }
        }
        width = w
        height = h
        self.rgb = rgb
    }

    func luminance(_ x: Int, _ y: Int) -> Double {
        let i = (y * width + x) * 3
        return 0.299 * Double(rgb[i]) + 0.587 * Double(rgb[i + 1]) + 0.114 * Double(rgb[i + 2])
    }
}

/// The S0.5 criteria for one frame against its reference.
struct Comparison: CustomStringConvertible {
    var wholeSSIM: Double
    /// Over pixels darker than luminance 180 in either frame, the wordmark's letters included.
    var inkSSIM: Double
    var inkPixels: Int
    /// The same outside the wordmark's box: the ink alone.
    var inkSSIMOutsideBox: Double
    /// Pixels where any channel differs by more than 8/255, outside the box.
    var offOutsideBox: Int
    /// The same, by luminance.
    var offOutsideBoxLuminance: Int
    var maxOutsideBox: Int

    init(_ ours: InkImage, _ reference: RGBImage, box: CGRect) {
        let w = ours.width, h = ours.height
        var a = [Double](repeating: 0, count: w * h), b = a
        var off = 0, offLuminance = 0, maxOff = 0
        for y in 0..<h {
            for x in 0..<w {
                a[y * w + x] = ours.luminance(x, y)
                b[y * w + x] = reference.luminance(x, y)
                guard !box.contains(CGPoint(x: Double(x) + 0.5, y: Double(y) + 0.5)) else { continue }
                let p = ours.pixel(x, y), i = (y * w + x) * 3
                let d = max(abs(Int(p.r) - Int(reference.rgb[i])), abs(Int(p.g) - Int(reference.rgb[i + 1])),
                            abs(Int(p.b) - Int(reference.rgb[i + 2])))
                maxOff = max(maxOff, d)
                if d > 8 { off += 1 }
                if abs(a[y * w + x] - b[y * w + x]) > 8 { offLuminance += 1 }
            }
        }
        let map = SSIM.map(a, b, width: w, height: h)
        wholeSSIM = map.mean
        // Ink: darker than luminance 180 in either frame (paper is about 230), as the S0.5 check.
        var sum = 0.0, count = 0, sumOutside = 0.0, countOutside = 0
        let r = SSIM.window / 2
        for y in 0..<map.height {
            for x in 0..<map.width where min(a[(y + r) * w + x + r], b[(y + r) * w + x + r]) < 180 {
                let v = map.values[y * map.width + x]
                sum += v
                count += 1
                if !box.contains(CGPoint(x: Double(x + r) + 0.5, y: Double(y + r) + 0.5)) {
                    sumOutside += v
                    countOutside += 1
                }
            }
        }
        inkSSIM = count > 0 ? sum / Double(count) : 1
        inkPixels = count
        inkSSIMOutsideBox = countOutside > 0 ? sumOutside / Double(countOutside) : 1
        offOutsideBox = off
        offOutsideBoxLuminance = offLuminance
        maxOutsideBox = maxOff
    }

    var description: String {
        String(format: "ssim=%.6f ssim_ink=%.6f (ink_px=%d) ssim_ink_outside_box=%.6f off>8_outside_box=%d (luminance %d) max_diff_outside_box=%d",
               wholeSSIM, inkSSIM, inkPixels, inkSSIMOutsideBox, offOutsideBox, offOutsideBoxLuminance, maxOutsideBox)
    }
}

/// Structural similarity, single scale.
enum SSIM {
    static let window = 11
    static let sigma = 1.5

    struct Map {
        var values: [Double]
        var width: Int
        var height: Int
        var mean: Double { values.reduce(0, +) / Double(values.count) }
    }

    static func map(_ x: [Double], _ y: [Double], width: Int, height: Int) -> Map {
        let c1: Double = (0.01 * 255) * (0.01 * 255), c2: Double = (0.03 * 255) * (0.03 * 255)
        let g = gaussian()
        let mu1 = filter(x, width, height, g), mu2 = filter(y, width, height, g)
        let xx = filter(x.map { $0 * $0 }, width, height, g)
        let yy = filter(y.map { $0 * $0 }, width, height, g)
        let xy = filter(zip(x, y).map { $0 * $1 }, width, height, g)
        var values = [Double](repeating: 0, count: mu1.count)
        for i in values.indices {
            let m1 = mu1[i], m2 = mu2[i]
            let s11 = xx[i] - m1 * m1, s22 = yy[i] - m2 * m2, s12 = xy[i] - m1 * m2
            values[i] = ((2 * m1 * m2 + c1) * (2 * s12 + c2)) / ((m1 * m1 + m2 * m2 + c1) * (s11 + s22 + c2))
        }
        return Map(values: values, width: width - window + 1, height: height - window + 1)
    }

    private static func gaussian() -> [Double] {
        let half = Double(window - 1) / 2
        let g = (0..<window).map { exp(-pow(Double($0) - half, 2) / (2 * sigma * sigma)) }
        let total = g.reduce(0, +)
        return g.map { $0 / total }
    }

    /// 'valid' correlation with outer(g, g), separably: (height - 10) x (width - 10).
    private static func filter(_ image: [Double], _ w: Int, _ h: Int, _ g: [Double]) -> [Double] {
        let k = g.count, ow = w - k + 1, oh = h - k + 1
        var rows = [Double](repeating: 0, count: oh * w)
        for y in 0..<oh {
            for x in 0..<w {
                var s = 0.0
                for i in 0..<k { s += g[i] * image[(y + i) * w + x] }
                rows[y * w + x] = s
            }
        }
        var out = [Double](repeating: 0, count: oh * ow)
        for y in 0..<oh {
            for x in 0..<ow {
                var s = 0.0
                for j in 0..<k { s += g[j] * rows[y * w + x + j] }
                out[y * ow + x] = s
            }
        }
        return out
    }
}
