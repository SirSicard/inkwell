// Generated from shaders/ink.wgsl by core/crates/ink-shader (naga). Do not edit: change the
// WGSL, then run `cargo run -p ink-shader --bin ink-shader` in core/.
//
// Shader model 5.0 (Direct3D 11, compiled at run time with D3DCompile). The uniform block G
// at b0, the only resource.
struct G {
    float2 res;
    float2 center_;
    float time;
    float unit;
    float you;
    float them;
    float4 w;
    float dark;
    float motion;
    float2 pad;
    float4 yA;
    float4 yB;
    float4 tA;
    float4 tB;
    float4 idle;
    float4 ink;
};

cbuffer g : register(b0) { G g; }

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
    float3 g_1 = float3(((a0_.x * x0_.x) + (h.x * x0_.y)), ((a0_.yz * _e120.xz) + (h.yz * _e124.yw)));
    float3 _e129 = m;
    return (130.0 * dot(_e129, g_1));
}

float fbm(float2 p_in)
{
    float2 p = (float2)0;
    float v_1 = 0.0;
    float a = 0.5;
    int i_1 = int(0);

    p = p_in;
    uint2 loop_bound = uint2(4294967295u, 4294967295u);
    bool loop_init = true;
    while(true) {
        if (all(loop_bound == uint2(0u, 0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            int _e28 = i_1;
            i_1 = asint(asuint(_e28) + asuint(int(1)));
        }
        loop_init = false;
        int _e8 = i_1;
        if ((_e8 < int(5))) {
        } else {
            break;
        }
        {
            float _e11 = v_1;
            float _e12 = a;
            float2 _e13 = p;
            const float _e14 = snoise(_e13);
            v_1 = (_e11 + (_e12 * _e14));
            float2 _e17 = p;
            p = ((_e17 * 2.03) + float2(1.7, 9.2));
            float _e24 = a;
            a = (_e24 * 0.5);
        }
    }
    float _e30 = v_1;
    return (0.48 + (0.455 * _e30));
}

float hash(float2 p_1)
{
    return frac((sin(dot(p_1, float2(12.9898, 78.233))) * 43758.547));
}

float4 orb(float2 q, float R, float soft, float3 A, float3 B, float seed, float tt)
{
    float3 c = (float3)0;

    float l = length(q);
    if ((l > ((R + soft) + 0.03))) {
        return (0.0).xxxx;
    }
    const float _e26 = fbm((((q * 3.0) + (seed).xx) + ((tt * 0.2)).xx));
    float a_1 = (1.0 - smoothstep((R - (soft * 0.5)), (R + soft), (l + (0.05 * (_e26 - 0.5)))));
    const float _e41 = fbm(((q * 2.2) + float2((tt * 0.15), seed)));
    c = lerp(A, B, smoothstep(0.3, 0.7, _e41));
    float d = length((q - (float2(-0.25, 0.3) * R)));
    float shine = ((R > 0.0) ? (1.0 - smoothstep(0.0, R, d)) : 1.0);
    float3 _e61 = c;
    float _e64 = g.dark;
    c = (_e61 + (((0.22 * (1.0 - (0.4 * _e64))) * shine)).xxx);
    float3 _e74 = c;
    return float4(_e74, a_1);
}

float3 greyed(float3 c_1, float k)
{
    return lerp(c_1, (dot(c_1, float3(0.299, 0.587, 0.114))).xxx, k);
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
    float4 o2_ = (0.0).xxxx;
    float3 col = (float3)0;

    float _e5 = g.center_.x;
    float _e10 = g.center_.y;
    float _e16 = g.unit;
    float2 p_2 = (float2((pos.x - _e5), (_e10 - pos.y)) / (_e16).xx);
    float dictating = g.w.x;
    float meeting = g.w.y;
    float blot = g.w.z;
    float problem = g.w.w;
    float live = max(dictating, meeting);
    float _e38 = g.time;
    float _e41 = g.motion;
    float t = (_e38 * _e41);
    float tt_1 = (t * (0.12 + (0.88 * live)));
    float apart = (meeting * (1.0 - blot));
    float2 swirl = (0.09 * float2(cos((tt_1 * 0.5)), sin((tt_1 * 0.5))));
    float2 c1_ = ((float2(-0.15, 0.02) + swirl) * apart);
    float2 c2_ = ((float2(0.15, -0.02) - swirl) * apart);
    float pulse = (0.5 + (0.5 * sin((t * 2.0))));
    float breath = (1.0 + ((0.06 * blot) * sin((t * 1.6))));
    float _e91 = g.you;
    float r1_ = lerp((((0.19 + (0.05 * dictating)) + ((0.09 * _e91) * live)) * lerp(0.8, 1.0, live)), (0.13 * breath), blot);
    float _e105 = g.them;
    float r2_ = (lerp(((0.16 + (0.08 * _e105)) * meeting), 0.0, blot) * (1.0 + ((0.06 * problem) * (pulse - 0.5))));
    float soft_1 = lerp(0.24, 0.025, blot);
    float _e127 = g.idle.w;
    float restTint = clamp(_e127, 0.0, 1.0);
    float4 _e133 = g.idle;
    float4 _e137 = g.yA;
    float3 restA = lerp(_e133.xyz, _e137.xyz, restTint);
    float4 _e142 = g.idle;
    float4 _e146 = g.tA;
    float3 restB = lerp(_e142.xyz, _e146.xyz, restTint);
    float4 _e152 = g.yA;
    float4 _e157 = g.ink;
    float4 _e162 = g.yB;
    float4 _e167 = g.ink;
    const float4 _e171 = orb((p_2 - c1_), r1_, soft_1, lerp(lerp(restA, _e152.xyz, live), _e157.xyz, blot), lerp(lerp(restB, _e162.xyz, live), _e167.xyz, blot), 0.0, tt_1);
    if ((meeting > 0.0)) {
        float4 _e180 = g.tA;
        const float3 _e184 = greyed(_e180.xyz, (0.85 * problem));
        float4 _e187 = g.ink;
        float4 _e192 = g.tB;
        const float3 _e196 = greyed(_e192.xyz, (0.85 * problem));
        float4 _e199 = g.ink;
        const float4 _e203 = orb((p_2 - c2_), r2_, soft_1, lerp(_e184, _e187.xyz, blot), lerp(_e196, _e199.xyz, blot), 3.0, tt_1);
        o2_ = _e203;
    }
    float _e205 = o2_.w;
    float a2_ = (((_e205 * 0.9) * meeting) * (1.0 - (problem * (0.45 - (0.3 * pulse)))));
    float _e220 = g.ink.w;
    float restAlpha = lerp(0.55, 0.825, clamp(_e220, 0.0, 1.0));
    float a1_ = (((_e171.w * 0.9) * lerp(restAlpha, 1.0, max(live, blot))) * (1.0 - ((0.35 * a2_) * (1.0 - blot))));
    float alpha = (a1_ + (a2_ * (1.0 - a1_)));
    if ((alpha <= 0.0)) {
        return (0.0).xxxx;
    }
    float4 _e252 = o2_;
    col = (((_e171.xyz * a1_) + ((_e252.xyz * a2_) * (1.0 - a1_))) / (max(alpha, 0.0001)).xxx);
    float3 _e264 = col;
    const float _e269 = hash((pos.xy + (frac(t)).xx));
    col = (_e264 + (((_e269 - 0.5) * 0.05)).xxx);
    float3 _e276 = col;
    col = clamp(_e276, (0.0).xxx, (1.0).xxx);
    float3 _e282 = col;
    return float4((_e282 * alpha), alpha);
}
