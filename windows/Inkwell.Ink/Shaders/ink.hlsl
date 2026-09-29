// Generated from shaders/ink.wgsl by core/crates/ink-shader (naga). Do not edit: change the
// WGSL, then run `cargo run -p ink-shader --bin ink-shader` in core/.
//
// Shader model 5.0 (Direct3D 11, compiled at run time with D3DCompile). The uniform block U
// at b0, the wordmark at t0, its sampler at s0.
struct U {
    float2 res;
    float time;
    float ampA;
    float ampB;
    float wet;
    float two;
    float dead;
    float blot;
    float breath;
    float cy;
    float hasMark;
    float4 drops[6];
};

struct Ctx {
    float T;
    int _pad1_0;
    float2 CC;
    float SS;
    int _end_pad_0;
};

cbuffer u : register(b0) { U u; }
Texture2D<float4> markTex : register(t0);
SamplerState markSamp : register(s0);

struct FragmentInput_fs_main {
    float4 pos_1 : SV_Position;
};

float2 mod289_2_(float2 x)
{
    return (x - (289.0 * floor((x / (289.0).xx))));
}

float3 mod289_3_(float3 x_1)
{
    return (x_1 - (289.0 * floor((x_1 / (289.0).xxx))));
}

float3 permute(float3 x_2)
{
    const float3 _e7 = mod289_3_((((x_2 * 34.0) + (1.0).xxx) * x_2));
    return _e7;
}

float snoise(float2 v)
{
    float2 i = (float2)0;
    float4 x12_ = (float4)0;
    float3 m = (float3)0;

    float4 C = float4(0.21132487, 0.36602542, -0.57735026, 0.024390243);
    i = floor((v + (dot(v, C.yy)).xx));
    float2 _e12 = i;
    float2 _e14 = i;
    float2 x0_ = ((v - _e12) + (dot(_e14, C.xx)).xx);
    float2 i1_ = ((x0_.x > x0_.y) ? float2(1.0, 0.0) : float2(0.0, 1.0));
    x12_ = (x0_.xyxy + C.xxzz);
    float4 _e33 = x12_;
    float4 _e36 = x12_;
    x12_ = float4((_e33.xy - i1_), _e36.zw);
    float2 _e39 = i;
    const float2 _e40 = mod289_2_(_e39);
    i = _e40;
    float _e42 = i.y;
    const float3 _e49 = permute(((_e42).xxx + float3(0.0, i1_.y, 1.0)));
    float _e51 = i.x;
    const float3 _e59 = permute(((_e49 + (_e51).xxx) + float3(0.0, i1_.x, 1.0)));
    float4 _e61 = x12_;
    float4 _e63 = x12_;
    float4 _e66 = x12_;
    float4 _e68 = x12_;
    m = max(((0.5).xxx - float3(dot(x0_, x0_), dot(_e61.xy, _e63.xy), dot(_e66.zw, _e68.zw))), (0.0).xxx);
    float3 _e79 = m;
    float3 _e80 = m;
    m = (_e79 * _e80);
    float3 _e82 = m;
    float3 _e83 = m;
    m = (_e82 * _e83);
    float3 x_3 = ((2.0 * frac((_e59 * C.www))) - (1.0).xxx);
    float3 h = (abs(x_3) - (0.5).xxx);
    float3 ox = floor((x_3 + (0.5).xxx));
    float3 a0_ = (x_3 - ox);
    float3 _e102 = m;
    m = (_e102 * ((1.7928429).xxx - (0.85373473 * ((a0_ * a0_) + (h * h)))));
    float4 _e120 = x12_;
    float4 _e124 = x12_;
    float3 g = float3(((a0_.x * x0_.x) + (h.x * x0_.y)), ((a0_.yz * _e120.xz) + (h.yz * _e124.yw)));
    float3 _e129 = m;
    return (130.0 * dot(_e129, g));
}

float fbmSoft(float2 p)
{
    const float _e1 = snoise(p);
    const float _e9 = snoise(((p * 2.0) + (5.2).xx));
    return ((_e1 * 0.68) + (_e9 * 0.32));
}

float fbm(float2 p_in)
{
    float2 p_1 = (float2)0;
    float v_1 = 0.0;
    float a = 0.5;
    int i_1 = int(0);

    p_1 = p_in;
    uint2 loop_bound = uint2(4294967295u, 4294967295u);
    bool loop_init = true;
    while(true) {
        if (all(loop_bound == uint2(0u, 0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            int _e24 = i_1;
            i_1 = asint(asuint(_e24) + asuint(int(1)));
        }
        loop_init = false;
        int _e8 = i_1;
        if ((_e8 < int(4))) {
        } else {
            break;
        }
        {
            float _e11 = v_1;
            float _e12 = a;
            float2 _e13 = p_1;
            const float _e14 = snoise(_e13);
            v_1 = (_e11 + (_e12 * _e14));
            float2 _e17 = p_1;
            p_1 = (_e17 * 2.03);
            float _e20 = a;
            a = (_e20 * 0.5);
        }
    }
    float _e26 = v_1;
    return _e26;
}

float hash(float2 p_2)
{
    return frac((sin(dot(p_2, float2(12.9898, 78.233))) * 43758.547));
}

float drop(float2 p_3, float2 c, float r)
{
    float2 d = (p_3 - c);
    return ((r * r) / max(dot(d, d), 1e-5));
}

float fibres(float2 p_4)
{
    const float _e3 = snoise((p_4 * 1.7));
    float a_1 = (_e3 * 1.4);
    float2 d_1 = float2(cos(a_1), sin(a_1));
    float2 q_2 = float2(dot(p_4, d_1), dot(p_4, float2(-(d_1.y), d_1.x)));
    const float _e23 = snoise(float2((q_2.x * 7.0), (q_2.y * 110.0)));
    return ((_e23 * 0.5) + 0.5);
}

float2 centreA(Ctx k_1)
{
    float _e5 = u.two;
    float _e10 = u.two;
    return (k_1.CC + (k_1.SS * float2((-0.1 * _e5), (0.05 * _e10))));
}

float2 centreB(Ctx k_2)
{
    return (k_2.CC + (k_2.SS * float2(0.19, -0.19)));
}

float fieldA(float2 q, Ctx k_3)
{
    float f = (float)0;
    int i_2 = int(0);
    bool local = (bool)0;

    const float2 _e2 = centreA(k_3);
    float _e5 = u.ampA;
    float _e10 = u.breath;
    float pulse = ((_e5 * 0.07) + (_e10 * 0.012));
    float _e16 = u.two;
    float shrink = (1.0 - (0.3 * _e16));
    float _e43 = u.two;
    const float _e49 = drop(q, (_e2 + (k_3.SS * float2((0.03 * sin((k_3.T * 0.9))), (0.03 * cos((k_3.T * 0.7)))))), ((k_3.SS * (0.205 + pulse)) * (1.0 - (0.12 * _e43))));
    f = _e49;
    float _e51 = f;
    const float _e81 = drop(q, (_e2 + (k_3.SS * float2((-0.2 + (0.05 * sin(((k_3.T * 1.1) + 1.0)))), (-0.17 + (0.04 * cos((k_3.T * 0.95))))))), ((k_3.SS * (0.085 + (pulse * 0.5))) * shrink));
    f = (_e51 + _e81);
    float _e83 = f;
    const float _e115 = drop(q, (_e2 + (k_3.SS * float2((0.16 + (0.04 * cos(((k_3.T * 0.85) + 2.0)))), (0.2 + (0.05 * sin(((k_3.T * 1.05) + 0.7))))))), ((k_3.SS * (0.07 + (pulse * 0.4))) * shrink));
    f = (_e83 + _e115);
    uint2 loop_bound_1 = uint2(4294967295u, 4294967295u);
    bool loop_init_1 = true;
    while(true) {
        if (all(loop_bound_1 == uint2(0u, 0u))) { break; }
        loop_bound_1 -= uint2(loop_bound_1.y == 0u, 1u);
        if (!loop_init_1) {
            int _e143 = i_2;
            i_2 = asint(asuint(_e143) + asuint(int(1)));
        }
        loop_init_1 = false;
        int _e119 = i_2;
        if ((_e119 < int(6))) {
        } else {
            break;
        }
        {
            int _e124 = i_2;
            float4 d_2 = u.drops[_e124];
            if ((d_2.z > 0.0)) {
                local = (d_2.w < 0.5);
            } else {
                local = false;
            }
            bool _e136 = local;
            if (_e136) {
                float _e137 = f;
                const float _e140 = drop(q, d_2.xy, d_2.z);
                f = (_e137 + _e140);
            }
        }
    }
    float _e145 = f;
    return _e145;
}

float fieldB(float2 q_1, Ctx k_4)
{
    float f_1 = (float)0;
    int i_3 = int(0);
    bool local_1 = (bool)0;

    float _e4 = u.two;
    if ((_e4 < 0.01)) {
        return 0.0;
    }
    const float2 _e8 = centreB(k_4);
    float _e11 = u.ampB;
    float _e16 = u.breath;
    float pulse_1 = ((_e11 * 0.07) + (_e16 * 0.01));
    float _e44 = u.two;
    const float _e46 = drop(q_1, (_e8 + (k_4.SS * float2((0.03 * cos(((k_4.T * 0.8) + 1.3))), (0.03 * sin((k_4.T * 0.6)))))), ((k_4.SS * (0.14 + pulse_1)) * _e44));
    f_1 = _e46;
    float _e48 = f_1;
    float _e77 = u.two;
    const float _e79 = drop(q_1, (_e8 + (k_4.SS * float2((0.11 + (0.04 * sin((k_4.T * 1.2)))), (-0.1 + (0.03 * cos((k_4.T * 0.9))))))), ((k_4.SS * (0.055 + (pulse_1 * 0.4))) * _e77));
    f_1 = (_e48 + _e79);
    uint2 loop_bound_2 = uint2(4294967295u, 4294967295u);
    bool loop_init_2 = true;
    while(true) {
        if (all(loop_bound_2 == uint2(0u, 0u))) { break; }
        loop_bound_2 -= uint2(loop_bound_2.y == 0u, 1u);
        if (!loop_init_2) {
            int _e111 = i_3;
            i_3 = asint(asuint(_e111) + asuint(int(1)));
        }
        loop_init_2 = false;
        int _e83 = i_3;
        if ((_e83 < int(6))) {
        } else {
            break;
        }
        {
            int _e88 = i_3;
            float4 d_3 = u.drops[_e88];
            if ((d_3.z > 0.0)) {
                local_1 = (d_3.w >= 0.5);
            } else {
                local_1 = false;
            }
            bool _e100 = local_1;
            if (_e100) {
                float _e101 = f_1;
                float _e106 = u.two;
                const float _e108 = drop(q_1, d_3.xy, (d_3.z * _e106));
                f_1 = (_e101 + _e108);
            }
        }
    }
    float _e113 = f_1;
    return _e113;
}

float zoneFade(float2 p_5, float aspect, Ctx k_5)
{
    float d_4 = min(min(p_5.x, (aspect - p_5.x)), min(p_5.y, (1.0 - p_5.y)));
    return smoothstep(0.0, (k_5.SS * 0.1), d_4);
}

float fence(float f_2, float fade)
{
    float g_1 = ((f_2 / (1.0 + f_2)) * fade);
    return ((fade >= 1.0) ? f_2 : (g_1 / (1.0 - g_1)));
}

float4 vs_main(uint vi : SV_VertexID) : SV_Position
{
    float x_4 = ((float((vi & 1u)) * 2.0) - 1.0);
    float y = ((float(((vi >> 1u) & 1u)) * 2.0) - 1.0);
    return float4(x_4, y, 0.0, 1.0);
}

float4 fs_main(FragmentInput_fs_main fragmentinput_fs_main) : SV_Target0
{
    float4 pos = fragmentinput_fs_main.pos_1;
    Ctx k = (Ctx)0;
    float3 paper = (float3)0;
    float3 colA = (float3)0;
    float3 colB = (float3)0;
    float3 col = (float3)0;
    float mk = 0.0;

    float _e5 = u.res.y;
    float2 frag = float2(pos.x, (_e5 - pos.y));
    float2 _e11 = u.res;
    float2 uv = (frag / _e11);
    float _e16 = u.res.y;
    float2 p_6 = (frag / (_e16).xx);
    float _e22 = u.res.x;
    float _e26 = u.res.y;
    float aspect_1 = (_e22 / _e26);
    k.SS = min(aspect_1, 1.0);
    float _e37 = u.cy;
    k.CC = float2((0.5 * aspect_1), _e37);
    float _e41 = u.ampA;
    float _e44 = u.ampB;
    float act = max(_e41, _e44);
    float _e49 = u.time;
    float _e52 = u.wet;
    k.T = (_e49 * ((0.12 + (0.3 * _e52)) + (0.35 * act)));
    float _e62 = k.SS;
    float _e68 = k.T;
    float _e72 = k.T;
    const float _e77 = fbmSoft((((p_6 / (_e62).xx) * 1.6) + float2((_e68 * 0.17), (_e72 * 0.1))));
    float _e79 = k.SS;
    float _e85 = k.T;
    float _e90 = k.T;
    const float _e98 = fbmSoft(((((p_6 / (_e79).xx) * 1.6) + float2((-(_e85) * 0.12), (_e90 * 0.15))) + (19.7).xx));
    float2 w = float2(_e77, _e98);
    float _e101 = k.SS;
    float _e105 = u.wet;
    float2 wp = (p_6 + ((w * _e101) * ((0.07 + (0.07 * _e105)) + (0.14 * act))));
    const float _e118 = fibres((frag / (180.0).xx));
    const float _e119 = hash(frag);
    Ctx _e120 = k;
    const float _e121 = zoneFade(p_6, aspect_1, _e120);
    Ctx _e122 = k;
    const float _e123 = fieldA(wp, _e122);
    const float _e124 = fence(_e123, _e121);
    Ctx _e125 = k;
    const float _e126 = fieldB(wp, _e125);
    const float _e127 = fence(_e126, _e121);
    float _e132 = u.blot;
    float feather = ((_e118 - 0.5) * (0.1 + (0.22 * _e132)));
    float eA = (_e124 + ((feather * smoothstep(0.55, 1.0, _e124)) * (1.0 - smoothstep(1.0, 1.25, _e124))));
    float eB = (_e127 + ((feather * smoothstep(0.55, 1.0, _e127)) * (1.0 - smoothstep(1.0, 1.25, _e127))));
    float _e162 = u.blot;
    float edge = (0.045 + (0.06 * _e162));
    float mA = smoothstep((1.0 - edge), (1.0 + edge), eA);
    float mB = smoothstep((1.0 - edge), (1.0 + edge), eB);
    float bleedA = (smoothstep((1.0 - (edge * 5.0)), (1.0 - edge), eA) * (1.0 - mA));
    float bleedB = (smoothstep((1.0 - (edge * 5.0)), (1.0 - edge), eB) * (1.0 - mB));
    paper = ((float3(0.949, 0.933, 0.902) * (0.986 + (0.028 * _e118))) - ((0.016 * _e119)).xxx);
    float3 _e211 = paper;
    paper = (_e211 * (1.0 - (0.06 * pow((length((uv - (0.5).xx)) * 1.15), 2.0))));
    float _e226 = k.SS;
    float _e232 = k.T;
    float _e236 = k.T;
    const float _e242 = fbm((((wp / (_e226).xx) * 3.2) + float2((_e232 * 0.2), (-(_e236) * 0.13))));
    float3 inkA = float3(0.086, 0.094, 0.122);
    float rimA = (1.0 - smoothstep(1.0, 1.9, _e124));
    colA = (lerp(((inkA * 1.9) + ((_e242 * 0.035)).xxx), (inkA * 0.72), rimA) * (0.9 + (0.1 * _e118)));
    float3 inkB = float3(0.494, 0.329, 0.192);
    float rimB = (1.0 - smoothstep(1.0, 1.9, _e127));
    colB = (lerp(((inkB * 1.16) + ((_e242 * 0.03)).xxx), (inkB * 0.7), rimB) * (0.92 + (0.08 * _e118)));
    float _e292 = k.SS;
    float e = (0.0025 * _e292);
    float hA = clamp((_e124 - 1.0), 0.0, 1.2);
    Ctx _e303 = k;
    const float _e304 = fieldA((wp + float2(e, 0.0)), _e303);
    const float _e305 = fence(_e304, _e121);
    float hx = (clamp((_e305 - 1.0), 0.0, 1.2) - hA);
    Ctx _e315 = k;
    const float _e316 = fieldA((wp + float2(0.0, e)), _e315);
    const float _e317 = fence(_e316, _e121);
    float hy = (clamp((_e317 - 1.0), 0.0, 1.2) - hA);
    float _e329 = k.SS;
    float _e336 = k.SS;
    float3 nrm = normalize(float3((((-(hx) / e) * 0.035) * _e329), (((-(hy) / e) * 0.035) * _e336), 1.0));
    float3 L = float3(-0.45112923, 0.5513802, 0.7017566);
    float spec = pow(max(dot(reflect(-(L), nrm), float3(0.0, 0.0, 1.0)), 0.0), 26.0);
    float _e358 = u.blot;
    float sheet = step(0.001, _e358);
    float _e364 = u.blot;
    float bx = (uv.x - ((_e364 * 1.5) - 0.25));
    float band = ((sheet * smoothstep(-0.2, 0.0, bx)) * (1.0 - smoothstep(0.0, 0.12, bx)));
    float ahead = lerp(1.0, smoothstep(-0.02, 0.1, bx), sheet);
    float3 _e385 = colA;
    float _e390 = u.wet;
    colA = (_e385 + (((((spec * 0.5) * _e390) * ahead) * mA)).xxx);
    float3 _e396 = colA;
    float3 _e397 = colA;
    float3 _e400 = colA;
    colA = lerp(_e396, (_e397 + ((((0.75).xxx - _e400) * 0.28) * _e118)), band);
    float3 _e407 = colB;
    float3 _e408 = colB;
    float3 _e411 = colB;
    colB = lerp(_e407, (_e408 + ((((0.8).xxx - _e411) * 0.28) * _e118)), band);
    float3 _e418 = paper;
    paper = (_e418 * (1.0 - (0.035 * band)));
    float3 seal = float3(0.698, 0.227, 0.149);
    Ctx _e428 = k;
    const float2 _e429 = centreB(_e428);
    float2 dB = (p_6 - _e429);
    float ang = atan2(dB.y, dB.x);
    float _e437 = k.T;
    float dash = step(0.5, frac(((ang * 1.9099) + (_e437 * 0.1))));
    float ring = (smoothstep(0.8, 0.97, _e127) - smoothstep(1.03, 1.2, _e127));
    float3 _e451 = paper;
    col = _e451;
    float3 _e453 = col;
    float3 _e454 = colB;
    float _e459 = u.dead;
    col = lerp(_e453, _e454, ((bleedB * 0.26) * (1.0 - _e459)));
    float3 _e464 = col;
    float3 _e465 = colB;
    float _e468 = u.dead;
    col = lerp(_e464, _e465, (mB * (1.0 - (0.9 * _e468))));
    float3 _e475 = col;
    float _e479 = u.dead;
    col = lerp(_e475, seal, (((ring * dash) * _e479) * 0.8));
    float _e485 = k.SS;
    float _e491 = k.T;
    const float _e496 = fbm((((wp / (_e485).xx) * 5.0) + ((_e491 * 0.3)).xx));
    float marble = smoothstep(0.35, 0.65, ((_e496 * 0.5) + 0.5));
    float3 _e504 = col;
    float3 _e505 = colA;
    col = lerp(_e504, _e505, (bleedA * 0.26));
    float3 _e509 = col;
    float3 _e510 = colA;
    float _e516 = u.dead;
    col = lerp(_e509, _e510, (mA * (1.0 - (((0.45 * mB) * marble) * (1.0 - _e516)))));
    float _e528 = u.hasMark;
    if ((_e528 > 0.5)) {
        float4 _e538 = markTex.Sample(markSamp, float2(uv.x, (1.0 - uv.y)));
        mk = _e538.w;
    }
    float3 _e540 = col;
    float3 _e543 = col;
    float _e545 = mk;
    col = lerp(_e540, ((1.0).xxx - _e543), _e545);
    float3 _e547 = col;
    return float4(_e547, 1.0);
}
