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
// ink.wgsl: the Glow orb. A soft orb for you and, in a meeting, a second one for the far end,
// circling each other while the meeting runs; when it ends, both blot down into one small ink drop.
// The shells draw it behind their content and in the Drop, over whatever is behind.
//
// The single source for every shell: core/crates/ink-shader translates it with naga into the
// Metal Shading Language (mac/Sources/InkRenderer/Resources/ink.msl, compiled by the app at run
// time) and into HLSL for the Windows shell (windows/Inkwell.Ink/Shaders/ink.hlsl). Edit this
// file, then regenerate:
//
//     cargo run -p ink-shader --bin ink-shader            (from core/)
//
// A test fails while either generated file is stale.
//
// It follows the design prototype's fragment shader (GLSL ES 1.0, WebGL 1). The differences:
//   1. The noise. The prototype's value noise is replaced by fbm() over snoise(), whose licence
//      notice heads this file: the only noise here. fbm() is scaled to the value noise's mean and
//      spread, so the edges and the colour bands move alike, though not pixel for pixel.
//   2. The grain hashes with hash(), not the prototype's hash.
//   3. gl_FragCoord has a bottom-left origin; WGSL @builtin(position), like `center`, has a
//      top-left one. p flips y, so it runs up as in the prototype.
//   4. GLSL's smoothstep with its edges reversed is undefined, in WGSL and MSL too. It is written
//      as 1 - smoothstep with the edges in order, which is what the prototype's GPU computed.
//   5. Additions: `motion` (0 stops time, for one still frame), the problem state (w.w: the far
//      end's orb fades toward grey and pulses slowly), and premultiplied output, clamped to 0..1
//      first as WebGL clamps it.
//
// Bind group 0 has one binding, which the generator maps to slot 0 of its kind ([[buffer(0)]] in
// the fragment stage; b0 in HLSL):
//   binding 0: uniform buffer G, 160 bytes. WGSL uniform layout (std140-compatible):
//     offset size field   meaning
//     0      8    res     the drawable's size in pixels (vec2)
//     8      8    center  the orb's centre in pixels, from the top left (vec2)
//     16     4    time    seconds
//     20     4    unit    pixels per orb unit (see Units)
//     24     4    you     your level, 0..1
//     28     4    them    the far end's level, 0..1
//     32     16   w       state weights, each 0..1 (vec4): x dictating, y meeting, z blotting,
//                         w problem
//     48     4    dark    1 in dark mode, 0 in light
//     52     4    motion  1 animates; 0 draws one still frame, whatever `time` is
//     56     8    pad     unused (vec2)
//     64     16   yA      your colour (vec4: rgb, a unused; so are the five below)
//     80     16   yB      your partner shade
//     96     16   tA      the far end's colour
//     112    16   tB      its partner shade
//     128    16   idle    the orb at rest
//     144    16   ink     the drop it blots down to
// Colours are sRGB, 0..1, written as they are: render into a UNORM target, not an sRGB one.
//
// Units: p is the pixel's offset from `center` divided by `unit`, with y up. At rest the orb's
// radius is 0.152 units, and its soft edge falls from full at 0.032 to nothing at 0.392.
//
// Output: premultiplied alpha, and (0, 0, 0, 0) wherever there is no orb. Blend it with one and
// one minus source alpha over what is behind.
//
// The vertex stage draws a full-screen quad as a 4-vertex triangle strip from vertex_index:
// (-1,-1), (1,-1), (-1,1), (1,1).

struct G {
    res: vec2<f32>,
    center: vec2<f32>,
    time: f32,
    unit: f32,
    you: f32,
    them: f32,
    w: vec4<f32>,
    dark: f32,
    motion: f32,
    pad: vec2<f32>,
    yA: vec4<f32>,
    yB: vec4<f32>,
    tA: vec4<f32>,
    tB: vec4<f32>,
    idle: vec4<f32>,
    ink: vec4<f32>,
}

@group(0) @binding(0) var<uniform> g: G;

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

// Five octaves of snoise, mapped to the prototype's value-noise fbm: mean 0.48, spread 0.124
// (snoise's five-octave sum has a spread of 0.272, so it is scaled by 0.455). It stays inside
// 0.04..0.92.
fn fbm(p_in: vec2<f32>) -> f32 {
    var p = p_in;
    var v = 0.0;
    var a = 0.5;
    for (var i = 0; i < 5; i++) {
        v += a * snoise(p);
        p = p * 2.03 + vec2<f32>(1.7, 9.2);
        a *= 0.5;
    }
    return 0.48 + 0.455 * v;
}

fn hash(p: vec2<f32>) -> f32 { return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453); }

// One orb at q (its own coordinates): colour A banding into B, a soft noisy edge at radius R, and
// a highlight up and to the left. rgb is the colour, a its coverage.
fn orb(q: vec2<f32>, R: f32, soft: f32, A: vec3<f32>, B: vec3<f32>, seed: f32, tt: f32) -> vec4<f32> {
    let l = length(q);
    // Past the soft edge by more than its noise can reach (0.05 * 0.46): nothing to draw, and no
    // noise to compute.
    if (l > R + soft + 0.03) { return vec4<f32>(0.0); }
    let a = 1.0 - smoothstep(R - soft * 0.5, R + soft, l + 0.05 * (fbm(q * 3.0 + seed + tt * 0.2) - 0.5));
    let h = fbm(q * 2.2 + vec2<f32>(tt * 0.15, seed));
    var c = mix(A, B, smoothstep(0.3, 0.7, h));
    // The prototype's smoothstep(R, 0, d): 1 at R = 0, as its GPU computed it.
    let d = length(q - vec2<f32>(-0.25, 0.3) * R);
    let shine = select(1.0, 1.0 - smoothstep(0.0, R, d), R > 0.0);
    c += 0.22 * (1.0 - 0.4 * g.dark) * shine;
    return vec4<f32>(c, a);
}

// The far end's colour, faded toward its own grey (the same luminance) by k.
fn greyed(c: vec3<f32>, k: f32) -> vec3<f32> {
    return mix(c, vec3<f32>(dot(c, vec3<f32>(0.299, 0.587, 0.114))), k);
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<f32>(pos.x - g.center.x, g.center.y - pos.y) / g.unit;
    let dictating = g.w.x;
    let meeting = g.w.y;
    let blot = g.w.z;
    let problem = g.w.w;
    let live = max(dictating, meeting);
    let t = g.time * g.motion;
    // Time runs slowly at rest and at full speed while live.
    let tt = t * (0.12 + 0.88 * live);
    // In a meeting the two orbs sit apart and circle; blotting brings them back together.
    let apart = meeting * (1.0 - blot);
    let swirl = 0.09 * vec2<f32>(cos(tt * 0.5), sin(tt * 0.5));
    let c1 = (vec2<f32>(-0.15, 0.02) + swirl) * apart;
    let c2 = (vec2<f32>(0.15, -0.02) - swirl) * apart;
    // The problem state's slow pulse: about once every three seconds.
    let pulse = 0.5 + 0.5 * sin(t * 2.0);
    let r1 = mix((0.19 + 0.05 * dictating + 0.09 * g.you * live) * mix(0.8, 1.0, live), 0.06, blot);
    let r2 = mix((0.16 + 0.08 * g.them) * meeting, 0.0, blot) * (1.0 + 0.06 * problem * (pulse - 0.5));
    let soft = mix(0.24, 0.012, blot);
    let o1 = orb(
        p - c1, r1, soft,
        mix(mix(g.idle.rgb, g.yA.rgb, live), g.ink.rgb, blot),
        mix(mix(g.idle.rgb, g.yB.rgb, live), g.ink.rgb, blot),
        0.0, tt,
    );
    var o2 = vec4<f32>(0.0);
    if (meeting > 0.0) {
        o2 = orb(
            p - c2, r2, soft,
            mix(greyed(g.tA.rgb, 0.85 * problem), g.ink.rgb, blot),
            mix(greyed(g.tB.rgb, 0.85 * problem), g.ink.rgb, blot),
            3.0, tt,
        );
    }
    // Yours over theirs. Theirs, in the problem state, between 55 % and 85 % of its strength.
    let a2 = o2.a * 0.9 * meeting * (1.0 - problem * (0.45 - 0.3 * pulse));
    let a1 = o1.a * 0.9 * mix(0.55, 1.0, max(live, blot)) * (1.0 - 0.35 * a2 * (1.0 - blot));
    let alpha = a1 + a2 * (1.0 - a1);
    if (alpha <= 0.0) { return vec4<f32>(0.0); }
    var col = (o1.rgb * a1 + o2.rgb * a2 * (1.0 - a1)) / max(alpha, 0.0001);
    // Grain, still when time is.
    col += (hash(pos.xy + fract(t)) - 0.5) * 0.05;
    col = clamp(col, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(col * alpha, alpha);
}
