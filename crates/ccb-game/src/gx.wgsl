// Emulates the GX TEV (texture environment) color combiner for one material.
// Stage configuration comes from a uniform, so a single shader serves every material.
// All math happens in gamma space like on the Wii; the result is linearized at the end.

#import bevy_pbr::forward_io::VertexOutput

struct GxParams {
    // x: stage count, y: packed alpha test, z: bit0 has tex0, bit1 has tex1
    info: vec4<u32>,
    color_env: array<vec4<u32>, 4>,
    alpha_env: array<vec4<u32>, 4>,
    // per stage: tex_map | tex_enabled << 4 | channel << 5 | kcsel << 8 | kasel << 16
    order: array<vec4<u32>, 4>,
    // PREV, C0, C1, C2 (may exceed 0..1: TEV registers are 10-bit signed)
    regs: array<vec4<f32>, 4>,
    konst: array<vec4<f32>, 4>,
    tex_mtx: array<vec4<f32>, 2>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> gx: GxParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var tex0: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var samp0: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var tex1: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var samp1: sampler;

fn word(a: array<vec4<u32>, 4>, i: u32) -> u32 {
    return a[i / 4u][i % 4u];
}

fn konst_frac(sel: u32) -> f32 {
    return f32(8u - sel) / 8.0;
}

fn konst_color(sel: u32) -> vec3<f32> {
    if (sel <= 7u) { return vec3(konst_frac(sel)); }
    if (sel >= 12u && sel <= 15u) { return gx.konst[sel - 12u].rgb; }
    if (sel >= 16u) {
        let k = gx.konst[(sel - 16u) % 4u];
        let comp = (sel - 16u) / 4u;
        return vec3(k[comp]);
    }
    return vec3(0.0);
}

fn konst_alpha(sel: u32) -> f32 {
    if (sel <= 7u) { return konst_frac(sel); }
    if (sel >= 16u) {
        let k = gx.konst[(sel - 16u) % 4u];
        return k[(sel - 16u) / 4u];
    }
    return 0.0;
}

fn bias_of(b: u32) -> f32 {
    if (b == 1u) { return 0.5; }
    if (b == 2u) { return -0.5; }
    return 0.0;
}

fn scale_of(s: u32) -> f32 {
    if (s == 1u) { return 2.0; }
    if (s == 2u) { return 4.0; }
    if (s == 3u) { return 0.5; }
    return 1.0;
}

fn alpha_cmp(f: u32, a: u32, r: u32) -> bool {
    switch f {
        case 0u: { return false; }
        case 1u: { return a < r; }
        case 2u: { return a == r; }
        case 3u: { return a <= r; }
        case 4u: { return a > r; }
        case 5u: { return a != r; }
        case 6u: { return a >= r; }
        default: { return true; }
    }
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3(2.4));
    return select(hi, lo, c <= vec3(0.04045));
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var uv = vec2(0.0);
#ifdef VERTEX_UVS_A
    uv = in.uv;
#endif
    uv = vec2(dot(gx.tex_mtx[0].xyz, vec3(uv, 1.0)), dot(gx.tex_mtx[1].xyz, vec3(uv, 1.0)));
    var ras = vec4(1.0);
#ifdef VERTEX_COLORS
    ras = in.color;
#endif
    // Sample up front: sampling must happen in uniform control flow.
    let t0 = textureSample(tex0, samp0, uv);
    let t1 = textureSample(tex1, samp1, uv);

    var r = gx.regs;
    var out_c = r[0].rgb;
    var out_a = r[0].a;
    let n = gx.info.x;
    for (var i = 0u; i < n; i++) {
        let ce = word(gx.color_env, i);
        let ae = word(gx.alpha_env, i);
        let o = word(gx.order, i);
        var tex = vec4(1.0);
        if (((o >> 4u) & 1u) == 1u) {
            tex = select(t1, t0, (o & 7u) == 0u);
        }
        let ch = (o >> 5u) & 7u;
        let rs = select(ras, vec4(0.0), ch == 7u);
        let kc = konst_color((o >> 8u) & 31u);
        let ka = konst_alpha((o >> 16u) & 31u);

        // Color inputs: CPREV APREV C0 A0 C1 A1 C2 A2 TEXC TEXA RASC RASA ONE HALF KONST ZERO
        var cin: array<vec3<f32>, 16>;
        cin[0] = r[0].rgb; cin[1] = vec3(r[0].a);
        cin[2] = r[1].rgb; cin[3] = vec3(r[1].a);
        cin[4] = r[2].rgb; cin[5] = vec3(r[2].a);
        cin[6] = r[3].rgb; cin[7] = vec3(r[3].a);
        cin[8] = tex.rgb; cin[9] = vec3(tex.a);
        cin[10] = rs.rgb; cin[11] = vec3(rs.a);
        cin[12] = vec3(1.0); cin[13] = vec3(0.5);
        cin[14] = kc; cin[15] = vec3(0.0);
        // Alpha inputs: APREV A0 A1 A2 TEXA RASA KONST ZERO
        var ain: array<f32, 8>;
        ain[0] = r[0].a; ain[1] = r[1].a; ain[2] = r[2].a; ain[3] = r[3].a;
        ain[4] = tex.a; ain[5] = rs.a; ain[6] = ka; ain[7] = 0.0;

        // The lerp operands a, b, c are 8-bit on hardware; clamp them to 0..1.
        let ca = clamp(cin[(ce >> 12u) & 15u], vec3(0.0), vec3(1.0));
        let cb = clamp(cin[(ce >> 8u) & 15u], vec3(0.0), vec3(1.0));
        let cc = clamp(cin[(ce >> 4u) & 15u], vec3(0.0), vec3(1.0));
        let cd = cin[ce & 15u];
        var c = mix(ca, cb, cc);
        if (((ce >> 18u) & 1u) == 1u) { c = -c; }
        c = (cd + c + bias_of((ce >> 16u) & 3u)) * scale_of((ce >> 20u) & 3u);
        if (((ce >> 19u) & 1u) == 1u) { c = clamp(c, vec3(0.0), vec3(1.0)); }

        let aa = clamp(ain[(ae >> 13u) & 7u], 0.0, 1.0);
        let ab = clamp(ain[(ae >> 10u) & 7u], 0.0, 1.0);
        let ac = clamp(ain[(ae >> 7u) & 7u], 0.0, 1.0);
        let ad = ain[(ae >> 4u) & 7u];
        var a = mix(aa, ab, ac);
        if (((ae >> 18u) & 1u) == 1u) { a = -a; }
        a = (ad + a + bias_of((ae >> 16u) & 3u)) * scale_of((ae >> 20u) & 3u);
        if (((ae >> 19u) & 1u) == 1u) { a = clamp(a, 0.0, 1.0); }

        let cdst = (ce >> 22u) & 3u;
        let adst = (ae >> 22u) & 3u;
        r[cdst] = vec4(c, r[cdst].a);
        r[adst].a = a;
        out_c = c;
        out_a = a;
    }

    out_c = clamp(out_c, vec3(0.0), vec3(1.0));
    out_a = clamp(out_a, 0.0, 1.0);
    let at = gx.info.y;
    let a8 = u32(round(out_a * 255.0));
    let p0 = alpha_cmp(at & 7u, a8, (at >> 8u) & 255u);
    let p1 = alpha_cmp((at >> 3u) & 7u, a8, (at >> 16u) & 255u);
    let op = (at >> 6u) & 3u;
    var pass_test = p0 && p1;
    if (op == 1u) { pass_test = p0 || p1; }
    if (op == 2u) { pass_test = p0 != p1; }
    if (op == 3u) { pass_test = p0 == p1; }
    if (!pass_test) {
        discard;
    }
    return vec4(srgb_to_linear(out_c), out_a);
}
