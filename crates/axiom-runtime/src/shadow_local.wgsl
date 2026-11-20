// Spot / point light shadow maps (shadow_local.rs), shared by cube and mesh shaders
// (bindings 3-5 of group 0). Faces 0..1 are the spot maps (layers of local_spot_shadow), faces
// 2..7 the point light's cube faces +X,-X,+Y,-Y,+Z,-Z (layers of local_point_shadow), each a
// 90-degree perspective map. LocalLight.params for spot / point: z = 1 + first face (0 = no
// shadow), w = world size of one shadow texel per meter from the light.
struct LocalShadowMatrices { face: array<mat4x4<f32>, 8> }
@group(0) @binding(3) var<uniform> local_shadow: LocalShadowMatrices;
@group(0) @binding(4) var local_spot_shadow: texture_depth_2d_array;
@group(0) @binding(5) var local_point_shadow: texture_depth_2d_array;
const LOCAL_SPOT_RES: f32 = 1024.0;
const LOCAL_POINT_RES: f32 = 512.0;
// Visibility of light `l` at world point p with normal n: 1 lit, 0 shadowed (3x3 PCF).
fn axiom_local_shadow(l: LocalLight, p: vec3<f32>, n: vec3<f32>) -> f32 {
    let kind = l.direction_kind.w;
    if kind > 1.5 || l.params.z < 0.5 { return 1.0; }
    let first = u32(l.params.z - 0.5);
    let to_light = l.position_range.xyz - p;
    let distance = length(to_light);
    // Normal offset plus a step toward the light, both one or two texels wide at this distance
    // (a linear-space bias: perspective depth is too uneven for a constant compare bias).
    let texel = distance * l.params.w;
    let q = p + n * (1.5 * texel) + to_light / max(distance, 0.0001) * texel;
    var face = first;
    var layer = 0;
    var res = LOCAL_SPOT_RES;
    if first >= 2u {
        let d = q - l.position_range.xyz;
        let a = abs(d);
        var side = 0u;
        if a.x >= a.y && a.x >= a.z { side = select(1u, 0u, d.x > 0.0); }
        else if a.y >= a.z { side = select(3u, 2u, d.y > 0.0); }
        else { side = select(5u, 4u, d.z > 0.0); }
        face = first + side;
        layer = i32(first - 2u + side);
        res = LOCAL_POINT_RES;
    } else {
        layer = i32(first);
    }
    let clip = local_shadow.face[face] * vec4<f32>(q, 1.0);
    if clip.w <= 0.0 { return 1.0; }
    let proj = clip.xyz / clip.w;
    let uv = proj.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || proj.z >= 1.0 { return 1.0; }
    var lit = 0.0;
    for (var x = -1; x <= 1; x = x + 1) {
        for (var y = -1; y <= 1; y = y + 1) {
            let at = uv + vec2<f32>(f32(x), f32(y)) / res;
            if first >= 2u {
                lit += textureSampleCompareLevel(local_point_shadow, shadow_sampler, at, layer, proj.z);
            } else {
                lit += textureSampleCompareLevel(local_spot_shadow, shadow_sampler, at, layer, proj.z);
            }
        }
    }
    return lit / 9.0;
}
