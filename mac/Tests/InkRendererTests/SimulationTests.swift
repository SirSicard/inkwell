// The ink's state machine and droplet physics against the design prototype's own JavaScript.
//
// The expected numbers were produced by running the prototype's `_step` (springs, envelope
// follower, droplet spawn and physics) with Math.random replaced by mulberry32, exactly as
// `InkRandom.seeded` does here. The port is doubles throughout, as JS is, so they agree to
// rounding: the tolerance is 1e-9.
import XCTest

@testable import InkRenderer

final class SimulationTests: XCTestCase {
    private let tolerance = 1e-9

    /// The fixed path (`applyFixed`) at t = 12: the values every reference render uses. At t = 12
    /// the synthetic voice is in a pause, so no state has any amplitude.
    func testTheFixedStateOfEveryStateAtT12() {
        let expected: [(InkState, wet: Double, two: Double, dead: Double, blot: Double)] = [
            (.idle, 0, 0, 0, 0),
            (.dictating, 1, 0, 0, 0),
            (.meeting, 0.65, 1, 0, 0),
            (.blotting, 0.45, 1, 0, 0.5),
            (.problem, 0.5, 1, 1, 0),
        ]
        for (state, wet, two, dead, blot) in expected {
            var sim = InkSimulation()
            sim.state = state
            sim.applyFixed(t: 12)
            XCTAssertEqual(sim.t, 12, "\(state): the fixed path does not advance time")
            XCTAssertEqual(sim.wet, wet, accuracy: tolerance, "\(state) wet")
            XCTAssertEqual(sim.two, two, accuracy: tolerance, "\(state) two")
            XCTAssertEqual(sim.dead, dead, accuracy: tolerance, "\(state) dead")
            XCTAssertEqual(sim.blot, blot, accuracy: tolerance, "\(state) blot")
            XCTAssertEqual(sim.envA, 0, "\(state) envA")
            XCTAssertEqual(sim.envB, 0, "\(state) envB")
            XCTAssertEqual(sim.breath, 0.706059, accuracy: 1e-6, "\(state) breath")
            XCTAssertFalse(sim.drops.contains(where: \.alive), "\(state): no droplets on the fixed path")
        }
    }

    /// t = 15.12 exercises the amplitude path: the near end speaks in three of the states.
    func testTheFixedStateAtT15_12CarriesTheVoice() {
        let expected: [(InkState, envA: Double)] = [
            (.idle, 0), (.dictating, 0.834168), (.meeting, 0.834168), (.blotting, 0), (.problem, 0.834168),
        ]
        for (state, envA) in expected {
            var sim = InkSimulation()
            sim.state = state
            sim.applyFixed(t: 15.12)
            XCTAssertEqual(sim.envA, envA, accuracy: 1e-6, "\(state) envA")
            XCTAssertEqual(sim.envB, 0, accuracy: 1e-12, "\(state) envB")
            XCTAssertEqual(sim.breath, 0.029365, accuracy: 1e-6, "\(state) breath")
        }
    }

    func testDictatingFor600LiveStepsMatchesThePrototype() {
        let sim = run(.dictating, steps: 600, seed: 1)
        expectState(sim, t: 21.999999999999858, wet: 0.9999999999999875, two: 0, dead: 0,
                    envA: 0.6144269784439061, envB: 0, coolA: 0.17, coolB: -10.000000000000076,
                    breath: 0.1441073288154759)
        XCTAssertEqual(sim.spawns, 12)
        expectDrops(sim, [
            .init(alive: true, x: 0.253184919774139, y: 0.4400177995046888, vx: -0.00754496737096397,
                  vy: 0.14209580701230884, r: 0.01856758185401559, ink: 0, age: 0.55),
            .init(alive: true, x: 0.17052380212789878, y: 0.5235585719183927, vx: -0.2848474047353476,
                  vy: 0.08443531837071983, r: 0.02075627201162279, ink: 0, age: 0.06666666666666667),
            .init(alive: true, x: 0.2511811072647956, y: 0.5041106485861104, vx: -0.02671918248261253,
                  vy: -0.0929916976788635, r: 0.003242740798292024, ink: 0, age: 1.0333333333333345),
            .init(), .init(), .init(),
        ])
    }

    /// 900 steps of a meeting: both inks throw droplets, and dead ones keep their last values, as
    /// the prototype's do.
    func testAMeetingFor900LiveStepsMatchesThePrototype() {
        let sim = run(.meeting, steps: 900, seed: 1)
        expectState(sim, t: 26.999999999999574, wet: 0.649999999999999, two: 0.999999999999999, dead: 0,
                    envA: 1.870380836993794e-12, envB: 0.007143259207834781, coolA: -4.346666666666657,
                    coolB: -0.4733333333333332, breath: 0.9927625557825327)
        XCTAssertEqual(sim.spawns, 18)
        expectDrops(sim, [
            .init(alive: false, x: 0.34918316573329206, y: 0.409301029980147, vx: -0.06352888333281757,
                  vy: -0.0653188635690675, r: 0.0018622836576670818, ink: 1, age: 0.9833333333333347),
            .init(alive: false, x: 0.34550002695732973, y: 0.405482857176386, vx: 0.06213916870072609,
                  vy: 0.06000545191008686, r: 0.00191059691410236, ink: 1, age: 1.100000000000001),
            .init(alive: true, x: 0.37462224794216203, y: 0.4021434750802657, vx: -0.15090867964798024,
                  vy: 0.01115891139625417, r: 0.007262791883202992, ink: 1, age: 0.7500000000000007),
            .init(alive: false, x: 0.349491018698219, y: 0.40366875882564107, vx: 0.06585327922175835,
                  vy: -0.019520425689014926, r: 0.0019319743033900463, ink: 1, age: 1.1500000000000008),
            .init(), .init(),
        ])
    }

    func testBlottingCyclesTheSheetAndThrowsNothing() {
        let sim = run(.blotting, steps: 600, seed: 1)
        XCTAssertEqual(sim.blot, 0.28985507246379605, accuracy: tolerance)
        XCTAssertEqual(sim.wet, 0.6669197833902668, accuracy: tolerance)
        XCTAssertEqual(sim.spawns, 0)
    }

    /// The droplets stay on the sheet: a loud, long meeting never puts one past the margins the
    /// physics clamps to.
    func testDropletsStayOnTheSheet() {
        for (w, h) in [(360.0, 720.0), (84.0, 84.0), (112.0, 1400.0), (900.0, 300.0)] {
            var sim = InkSimulation(random: .seeded(3))
            sim.canvasWidth = w
            sim.canvasHeight = h
            sim.state = .meeting
            let asp = w / h, s = min(asp, 1)
            for step in 0..<3000 {
                sim.step(1 / 60, snap: false, voice: .levels(near: 1, far: step % 40 < 20 ? 1 : 0))
                for d in sim.drops where d.alive {
                    XCTAssert(d.x >= s * 0.1 - 1e-12 && d.x <= asp - s * 0.1 + 1e-12, "x \(d.x) at \(w)x\(h)")
                    XCTAssert(d.y >= 0.06 - 1e-12 && d.y <= 0.94 + 1e-12, "y \(d.y) at \(w)x\(h)")
                }
            }
            XCTAssertGreaterThan(sim.spawns, 10, "the loud meeting threw droplets at \(w)x\(h)")
        }
    }

    /// Live input drives the envelopes by the state's rules: the near end in dictation, meetings
    /// and problems; the far end only in a meeting; nothing while idle or blotting.
    func testLevelsDriveTheEnvelopesOnlyWhereTheStateListens() {
        let cases: [(InkState, a: Bool, b: Bool)] = [
            (.idle, false, false), (.dictating, true, false), (.meeting, true, true),
            (.blotting, false, false), (.problem, true, false),
        ]
        for (state, a, b) in cases {
            var sim = InkSimulation(random: .seeded(1))
            sim.state = state
            sim.step(0, snap: true, voice: .levels(near: 0.8, far: 0.6))
            XCTAssertEqual(sim.envA, a ? 0.8 : 0, accuracy: 1e-12, "\(state) near end")
            XCTAssertEqual(sim.envB, b ? 0.6 : 0, accuracy: 1e-12, "\(state) far end")
        }
    }

    /// Entering idle clears the droplets and settles every spring: the still frame is the rest
    /// state, whatever was moving before.
    func testSettlingForAStillFrameClearsDropletsAndSnapsTheSprings() {
        var sim = run(.dictating, steps: 600, seed: 1)
        XCTAssertTrue(sim.drops.contains(where: \.alive))
        sim.state = .idle
        let t = sim.t
        sim.settle(voice: .silent)
        XCTAssertFalse(sim.drops.contains(where: \.alive))
        XCTAssertEqual(sim.wet, 0)
        XCTAssertEqual(sim.envA, 0)
        XCTAssertEqual(sim.t, t, "a still frame does not move time")
    }

    func testTheUniformBlockMatchesTheShader() {
        XCTAssertEqual(MemoryLayout<InkUniforms>.size, 144, "shaders/ink.wgsl: U is 144 bytes")
        XCTAssertEqual(MemoryLayout<InkUniforms>.stride, 144)
        XCTAssertEqual(MemoryLayout<InkUniforms>.offset(of: \.drops), 48)

        var sim = run(.meeting, steps: 900, seed: 1)
        sim.cy = 0.4
        let u = sim.uniforms(hasMark: true)
        XCTAssertEqual(u.res, SIMD2(360, 720))
        XCTAssertEqual(u.time, Float(sim.t))
        XCTAssertEqual(u.ampB, Float(sim.envB))
        XCTAssertEqual(u.cy, 0.4)
        XCTAssertEqual(u.hasMark, 1)
        // Only live droplets reach the shader; a dead slot is all zeros (r = 0 is dead there).
        XCTAssertEqual(u.drops.0, .zero)
        XCTAssertEqual(u.drops.2, SIMD4(Float(sim.drops[2].x), Float(sim.drops[2].y), Float(sim.drops[2].r), 1))
    }

    func testMulberry32MatchesItsReferenceSequence() {
        // The first draws of mulberry32(1), from its JavaScript definition.
        var rng = InkRandom.seeded(1)
        let first = (0..<3).map { _ in rng.next() }
        XCTAssertEqual(first[0], 0.6270739405881613, accuracy: 1e-15)
        XCTAssertEqual(first[1], 0.002735721180215478, accuracy: 1e-15)
        XCTAssertEqual(first[2], 0.5274470399599522, accuracy: 1e-15)
    }

    // MARK: Helpers

    private func run(_ state: InkState, steps: Int, seed: UInt32) -> InkSimulation {
        var sim = InkSimulation(random: .seeded(seed))
        sim.state = state
        sim.t = 12
        for _ in 0..<steps {
            sim.step(1.0 / 60.0, snap: false, voice: .synthetic)
        }
        return sim
    }

    private func expectState(
        _ sim: InkSimulation, t: Double, wet: Double, two: Double, dead: Double, envA: Double, envB: Double,
        coolA: Double, coolB: Double, breath: Double, line: UInt = #line
    ) {
        XCTAssertEqual(sim.t, t, accuracy: tolerance, "t", line: line)
        XCTAssertEqual(sim.wet, wet, accuracy: tolerance, "wet", line: line)
        XCTAssertEqual(sim.two, two, accuracy: tolerance, "two", line: line)
        XCTAssertEqual(sim.dead, dead, accuracy: tolerance, "dead", line: line)
        XCTAssertEqual(sim.envA, envA, accuracy: tolerance, "envA", line: line)
        XCTAssertEqual(sim.envB, envB, accuracy: tolerance, "envB", line: line)
        XCTAssertEqual(sim.coolA, coolA, accuracy: tolerance, "coolA", line: line)
        XCTAssertEqual(sim.coolB, coolB, accuracy: tolerance, "coolB", line: line)
        XCTAssertEqual(sim.breath, breath, accuracy: tolerance, "breath", line: line)
    }

    private func expectDrops(_ sim: InkSimulation, _ expected: [InkDroplet], line: UInt = #line) {
        XCTAssertEqual(sim.drops.count, expected.count, line: line)
        for (i, (got, want)) in zip(sim.drops, expected).enumerated() {
            XCTAssertEqual(got.alive, want.alive, "drop \(i) alive", line: line)
            XCTAssertEqual(got.ink, want.ink, "drop \(i) ink", line: line)
            for (name, g, w) in [("x", got.x, want.x), ("y", got.y, want.y), ("vx", got.vx, want.vx),
                                 ("vy", got.vy, want.vy), ("r", got.r, want.r), ("age", got.age, want.age)] {
                XCTAssertEqual(g, w, accuracy: tolerance, "drop \(i) \(name)", line: line)
            }
        }
    }
}
