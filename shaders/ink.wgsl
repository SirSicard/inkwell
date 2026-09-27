// Licence notice: snoise() below is 2D simplex noise from webgl-noise, by Ian McEwan, Ashima Arts
// (https://github.com/ashima/webgl-noise, maintained at https://github.com/stegu/webgl-noise), MIT
// licensed; ported here from GLSL to WGSL. Its notice ships in the Mac app's About screen.
//
// Copyright (C) 2011 by Ashima Arts (Simplex noise)
// Copyright (C) 2011-2016 by Stefan Gustavson (Classic noise and others)
// Permission is hereby granted, free of charge, to any person obtaining a copy of this software and
// associated documentation files (the "Software"), to deal in the Software without restriction,
// including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense,
// and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so,
// subject to the following conditions:
// The above copyright notice and this permission notice shall be included in all copies or substantial
// portions of the Software.
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT
// LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN
// NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY,
// WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE
// SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
//
// ink.wgsl: the ink. Two inks on paper (you in black, the far end in sepia), wet or dry, with a
// blotting sheet and a wordmark knocked out of whatever sits under it.
//
// The single source for every shell: core/crates/ink-shader translates it with naga into the
// Metal Shading Language (mac/Sources/InkRenderer/Resources/ink.msl, compiled by the app at run
// time), and for the Windows shell (S3.4) into HLSL. Edit this file, then regenerate:
//
//     cargo run -p ink-shader --bin ink-shader            (from core/)
//
// A test fails while the generated MSL is stale.
//
// It is a line-for-line port of the design prototype's fragment shader (GLSL ES 1.0, WebGL 1).
// Every difference below is a syntax or convention change that leaves the result identical:
//   1. GLSL mod(x, y) is x - y * floor(x / y). WGSL's `%` truncates toward zero instead, so
//      mod289_2/mod289_3 spell out the GLSL definition.
//   2. gl_FragCoord has a bottom-left origin; WGSL @builtin(position) has a top-left one.
//      frag = (pos.x, res.y - pos.y) restores the GLSL value, pixel centres included (both sit
//      at +0.5).
//   3. The GLSL globals T, CC and SS (assigned in main) become the Ctx struct, passed to every
//      function that reads them.
//   4. WGSL cannot assign to a multi-component swizzle, so `x12.xy -= i1` and `g.yz = ...`
//      rebuild the whole vector.
//   5. GLSL atan(y, x) is atan2(y, x); texture2D(...) is textureSample(...); the ternary is
//      select(false_value, true_value, cond).
// One addition, which the prototype does not have: zoneFade and fence keep the ink inside its zone
// (see there). Where the ink is clear of the zone's edge, as in every state at rest, it changes no
// pixel.
//
// Bind group 0 (the generator maps each binding to slot 0 of its kind: [[buffer(0)]],
// [[texture(0)]], [[sampler(0)]] in the fragment stage):
//   binding 0: uniform buffer U, 144 bytes. WGSL uniform layout (std140-compatible):
//     offset size field    GLSL name  meaning
//     0      8    res      uRes       canvas size in pixels (vec2)
//     8      4    time     uTime      seconds
//     12     4    ampA     uAmpA      envelope, near end (mic)
//     16     4    ampB     uAmpB      envelope, far end
//     20     4    wet      uWet       0..1
//     24     4    two      uTwo       0..1, second ink present
//     28     4    dead     uDead      0..1, far end silent (problem)
//     32     4    blot     uBlot      0..1, blotting sheet position
//     36     4    breath   uBreath    0..1
//     40     4    cy       uCY        body centre y, in p units
//     44     4    hasMark  uHasMark   > 0.5 draws the wordmark
//     48     96   drops    uDrops[6]  vec4 (x, y, r, ink) in p units; r = 0 is dead;
//                                     ink < 0.5 is ink A, >= 0.5 is ink B
//   binding 1: markTex, texture_2d<f32>. Alpha = wordmark coverage, row 0 = top.
//              Same pixel size as the canvas.
//   binding 2: markSamp, linear min/mag, clamp to edge.
//
// Units: p is a pixel position divided by the canvas height, so y runs 0..1 (up) and x runs
// 0..aspect. SS = min(aspect, 1) scales the ink to the zone's shorter side.
//
// The vertex stage draws a full-screen quad as a 4-vertex triangle strip from vertex_index:
// (-1,-1), (1,-1), (-1,1), (1,1).

struct U {
    res: vec2<f32>,
    time: f32,
    ampA: f32,
    ampB: f32,
    wet: f32,
    two: f32,
    dead: f32,
    blot: f32,
    breath: f32,
    cy: f32,
    hasMark: f32,
    drops: array<vec4<f32>, 6>,
}

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var markTex: texture_2d<f32>;
@group(0) @binding(2) var markSamp: sampler;

// The GLSL globals assigned in main().
struct Ctx {
    T: f32,
    CC: vec2<f32>,
    SS: f32,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let x = f32(vi & 1u) * 2.0 - 1.0;
    let y = f32((vi >> 1u) & 1u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

fn mod289_2(x: vec2<f32>) -> vec2<f32> { return x - 289.0 * floor(x / 289.0); }
fn mod289_3(x: vec3<f32>) -> vec3<f32> { return x - 289.0 * floor(x / 289.0); }

fn permute(x: vec3<f32>) -> vec3<f32> { return mod289_3(((x * 34.0) + 1.0) * x); }

fn snoise(v: vec2<f32>) -> f32 {
    let C = vec4<f32>(0.211324865405187, 0.366025403784439, -0.577350269189626, 0.024390243902439);
    var i = floor(v + dot(v, C.yy));
    let x0 = v - i + dot(i, C.xx);
    let i1 = select(vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), x0.x > x0.y);
    var x12 = x0.xyxy + C.xxzz;
    x12 = vec4<f32>(x12.xy - i1, x12.zw);
    i = mod289_2(i);
    let p = permute(permute(i.y + vec3<f32>(0.0, i1.y, 1.0)) + i.x + vec3<f32>(0.0, i1.x, 1.0));
    var m = max(0.5 - vec3<f32>(dot(x0, x0), dot(x12.xy, x12.xy), dot(x12.zw, x12.zw)), vec3<f32>(0.0));
    m = m * m;
    m = m * m;
    let x = 2.0 * fract(p * C.www) - 1.0;
    let h = abs(x) - 0.5;
    let ox = floor(x + 0.5);
    let a0 = x - ox;
    m *= 1.79284291400159 - 0.85373472095314 * (a0 * a0 + h * h);
    let g = vec3<f32>(a0.x * x0.x + h.x * x0.y, a0.yz * x12.xz + h.yz * x12.yw);
    return 130.0 * dot(m, g);
}

fn fbmSoft(p: vec2<f32>) -> f32 { return snoise(p) * 0.68 + snoise(p * 2.0 + 5.2) * 0.32; }

fn fbm(p_in: vec2<f32>) -> f32 {
    var p = p_in;
    var v = 0.0;
    var a = 0.5;
    for (var i = 0; i < 4; i++) {
        v += a * snoise(p);
        p *= 2.03;
        a *= 0.5;
    }
    return v;
}

fn hash(p: vec2<f32>) -> f32 { return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453); }

fn drop(p: vec2<f32>, c: vec2<f32>, r: f32) -> f32 {
    let d = p - c;
    return (r * r) / max(dot(d, d), 1e-5);
}

// Paper fibres: short strands at a slowly turning angle. Ink feathers along them.
fn fibres(p: vec2<f32>) -> f32 {
    let a = snoise(p * 1.7) * 1.4;
    let d = vec2<f32>(cos(a), sin(a));
    let q = vec2<f32>(dot(p, d), dot(p, vec2<f32>(-d.y, d.x)));
    return snoise(vec2<f32>(q.x * 7.0, q.y * 110.0)) * 0.5 + 0.5;
}

fn centreA(k: Ctx) -> vec2<f32> { return k.CC + k.SS * vec2<f32>(-0.10 * u.two, 0.05 * u.two); }
fn centreB(k: Ctx) -> vec2<f32> { return k.CC + k.SS * vec2<f32>(0.19, -0.19); }

fn fieldA(q: vec2<f32>, k: Ctx) -> f32 {
    let c = centreA(k);
    let pulse = u.ampA * 0.07 + u.breath * 0.012;
    let shrink = 1.0 - 0.3 * u.two;
    var f = drop(q, c + k.SS * vec2<f32>(0.03 * sin(k.T * 0.9), 0.03 * cos(k.T * 0.7)), k.SS * (0.205 + pulse) * (1.0 - 0.12 * u.two));
    f += drop(q, c + k.SS * vec2<f32>(-0.2 + 0.05 * sin(k.T * 1.1 + 1.0), -0.17 + 0.04 * cos(k.T * 0.95)), k.SS * (0.085 + pulse * 0.5) * shrink);
    f += drop(q, c + k.SS * vec2<f32>(0.16 + 0.04 * cos(k.T * 0.85 + 2.0), 0.2 + 0.05 * sin(k.T * 1.05 + 0.7)), k.SS * (0.07 + pulse * 0.4) * shrink);
    for (var i = 0; i < 6; i++) {
        let d = u.drops[i];
        if (d.z > 0.0 && d.w < 0.5) { f += drop(q, d.xy, d.z); }
    }
    return f;
}

fn fieldB(q: vec2<f32>, k: Ctx) -> f32 {
    if (u.two < 0.01) { return 0.0; }
    let c = centreB(k);
    let pulse = u.ampB * 0.07 + u.breath * 0.01;
    var f = drop(q, c + k.SS * vec2<f32>(0.03 * cos(k.T * 0.8 + 1.3), 0.03 * sin(k.T * 0.6)), k.SS * (0.14 + pulse) * u.two);
    f += drop(q, c + k.SS * vec2<f32>(0.11 + 0.04 * sin(k.T * 1.2), -0.1 + 0.03 * cos(k.T * 0.9)), k.SS * (0.055 + pulse * 0.4) * u.two);
    for (var i = 0; i < 6; i++) {
        let d = u.drops[i];
        if (d.z > 0.0 && d.w >= 0.5) { f += drop(q, d.xy, d.z * u.two); }
    }
    return f;
}

// The ink stays inside its zone. A zone narrower than the wet, loud ink (the 56 pt rail, a small
// panel) would otherwise cut the blob with a straight edge where the zone ends. zoneFade falls
// from 1 to 0 across a band along the zone's four edges, SS * 0.1 wide, and fence applies it to a
// field: it squashes the field below 1 (f / (1 + f)), scales that by the fade and maps it back.
// However strong the field, the result reaches the ink's edge (1) only where the fade is at least
// 0.5, so no ink comes nearer to the zone's edge than half the band, and a blob pushed that far
// thins and rounds off instead of being cut. Where the fade is 1 the field is returned unchanged,
// so ink clear of the band renders exactly as in the prototype.
fn zoneFade(p: vec2<f32>, aspect: f32, k: Ctx) -> f32 {
    let d = min(min(p.x, aspect - p.x), min(p.y, 1.0 - p.y));
    return smoothstep(0.0, k.SS * 0.1, d);
}

fn fence(f: f32, fade: f32) -> f32 {
    let g = f / (1.0 + f) * fade;
    return select(g / (1.0 - g), f, fade >= 1.0);
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let frag = vec2<f32>(pos.x, u.res.y - pos.y);
    let uv = frag / u.res;
    let p = frag / u.res.y;
    let aspect = u.res.x / u.res.y;
    var k: Ctx;
    k.SS = min(aspect, 1.0);
    k.CC = vec2<f32>(0.5 * aspect, u.cy);
    let act = max(u.ampA, u.ampB);
    k.T = u.time * (0.12 + 0.3 * u.wet + 0.35 * act);
    let w = vec2<f32>(fbmSoft(p / k.SS * 1.6 + vec2<f32>(k.T * 0.17, k.T * 0.1)), fbmSoft(p / k.SS * 1.6 + vec2<f32>(-k.T * 0.12, k.T * 0.15) + 19.7));
    let wp = p + w * k.SS * (0.07 + 0.07 * u.wet + 0.14 * act);
    let fib = fibres(frag / 180.0);
    let grain = hash(frag);
    let fade = zoneFade(p, aspect, k);
    let fA = fence(fieldA(wp, k), fade);
    let fB = fence(fieldB(wp, k), fade);
    // Capillary feathering, only in the thin band just outside the edge; strongest while blotting.
    let feather = (fib - 0.5) * (0.1 + 0.22 * u.blot);
    let eA = fA + feather * smoothstep(0.55, 1.0, fA) * (1.0 - smoothstep(1.0, 1.25, fA));
    let eB = fB + feather * smoothstep(0.55, 1.0, fB) * (1.0 - smoothstep(1.0, 1.25, fB));
    let edge = 0.045 + 0.06 * u.blot;
    let mA = smoothstep(1.0 - edge, 1.0 + edge, eA);
    let mB = smoothstep(1.0 - edge, 1.0 + edge, eB);
    let bleedA = smoothstep(1.0 - edge * 5.0, 1.0 - edge, eA) * (1.0 - mA);
    let bleedB = smoothstep(1.0 - edge * 5.0, 1.0 - edge, eB) * (1.0 - mB);
    var paper = vec3<f32>(0.949, 0.933, 0.902) * (0.986 + 0.028 * fib) - 0.016 * grain;
    paper *= 1.0 - 0.06 * pow(length(uv - 0.5) * 1.15, 2.0);
    // Pigment pools at the rim and runs a little thinner inside, like a real drop drying.
    let flow = fbm(wp / k.SS * 3.2 + vec2<f32>(k.T * 0.2, -k.T * 0.13));
    let inkA = vec3<f32>(0.086, 0.094, 0.122);
    let rimA = 1.0 - smoothstep(1.0, 1.9, fA);
    var colA = mix(inkA * 1.9 + flow * 0.035, inkA * 0.72, rimA) * (0.9 + 0.1 * fib);
    let inkB = vec3<f32>(0.494, 0.329, 0.192);
    let rimB = 1.0 - smoothstep(1.0, 1.9, fB);
    var colB = mix(inkB * 1.16 + flow * 0.03, inkB * 0.7, rimB) * (0.92 + 0.08 * fib);
    // Wet sheen: a highlight off the slope of the drop, gone once the ink is dry.
    let e = 0.0025 * k.SS;
    let hA = clamp(fA - 1.0, 0.0, 1.2);
    // The slope's two samples reuse this pixel's fade on purpose: they sit 0.0025 SS away, where
    // the fade differs by a negligible amount, and one fade keeps the slope the field's own.
    let hx = clamp(fence(fieldA(wp + vec2<f32>(e, 0.0), k), fade) - 1.0, 0.0, 1.2) - hA;
    let hy = clamp(fence(fieldA(wp + vec2<f32>(0.0, e), k), fade) - 1.0, 0.0, 1.2) - hA;
    let nrm = normalize(vec3<f32>(-hx / e * 0.035 * k.SS, -hy / e * 0.035 * k.SS, 1.0));
    let L = normalize(vec3<f32>(-0.45, 0.55, 0.7));
    let spec = pow(max(dot(reflect(-L, nrm), vec3<f32>(0.0, 0.0, 1.0)), 0.0), 26.0);
    // Blotting: a sheet passes left to right, lifting pigment and printing the paper into the ink.
    let sheet = step(0.001, u.blot);
    let bx = uv.x - (u.blot * 1.5 - 0.25);
    let band = sheet * smoothstep(-0.2, 0.0, bx) * (1.0 - smoothstep(0.0, 0.12, bx));
    let ahead = mix(1.0, smoothstep(-0.02, 0.1, bx), sheet);
    colA += spec * 0.5 * u.wet * ahead * mA;
    colA = mix(colA, colA + (vec3<f32>(0.75) - colA) * 0.28 * fib, band);
    colB = mix(colB, colB + (vec3<f32>(0.8) - colB) * 0.28 * fib, band);
    paper *= 1.0 - 0.035 * band;
    // The far end gone silent: its ink fades to a ghost with a dashed seal-red outline.
    let seal = vec3<f32>(0.698, 0.227, 0.149);
    let dB = p - centreB(k);
    let ang = atan2(dB.y, dB.x);
    let dash = step(0.5, fract(ang * 1.9099 + k.T * 0.1));
    let ring = smoothstep(0.8, 0.97, fB) - smoothstep(1.03, 1.2, fB);
    var col = paper;
    col = mix(col, colB, bleedB * 0.26 * (1.0 - u.dead));
    col = mix(col, colB, mB * (1.0 - 0.9 * u.dead));
    col = mix(col, seal, ring * dash * u.dead * 0.8);
    let marble = smoothstep(0.35, 0.65, fbm(wp / k.SS * 5.0 + k.T * 0.3) * 0.5 + 0.5);
    col = mix(col, colA, bleedA * 0.26);
    col = mix(col, colA, mA * (1.0 - 0.45 * mB * marble * (1.0 - u.dead)));
    // The wordmark is knocked out of whatever sits under it: dark on paper, light on ink.
    var mk = 0.0;
    if (u.hasMark > 0.5) { mk = textureSample(markTex, markSamp, vec2<f32>(uv.x, 1.0 - uv.y)).a; }
    col = mix(col, vec3<f32>(1.0) - col, mk);
    return vec4<f32>(col, 1.0);
}
