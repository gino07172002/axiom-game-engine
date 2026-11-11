// Sun shadow map (sun 0), shared by cube and mesh shaders (bindings 1 and 2 of group 0).
@group(0) @binding(1) var shadow_map: texture_depth_2d;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;
fn scene_shadow(light: vec4<f32>) -> f32 {
    let proj=light.xyz/light.w;let uv=proj.xy*vec2<f32>(0.5,-0.5)+vec2<f32>(0.5);
    var shadow=1.0;
    if all(uv>vec2<f32>(0.0)) && all(uv<vec2<f32>(1.0)) && proj.z<1.0 {
        shadow=0.0;
        for(var x=-1;x<=1;x=x+1) {for(var y=-1;y<=1;y=y+1) {
            shadow+=textureSampleCompareLevel(shadow_map,shadow_sampler,uv+vec2<f32>(f32(x),f32(y))/2048.0,proj.z-0.0018)/9.0;
        }}
    }
    return shadow;
}
// The shadow map belongs to sun 0 when its direction_shadow.w is set.
fn sun_shadow(light: vec4<f32>) -> f32 {
    if g.suns[0].direction_shadow.w > 0.5 { return scene_shadow(light); }
    return 1.0;
}
