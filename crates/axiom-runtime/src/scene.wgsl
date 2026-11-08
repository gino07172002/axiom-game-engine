struct LocalLight {position_radius:vec4<f32>,color_intensity:vec4<f32>}
struct Globals { view_proj: mat4x4<f32>, light_vp: mat4x4<f32>, eye_time: vec4<f32>, accent: vec4<f32>, lights:array<LocalLight,32>, view_params:vec4<f32> }
@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var shadow_map: texture_depth_2d;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;
struct In {
    @location(0) position: vec3<f32>, @location(1) normal: vec3<f32>,
    @location(2) m0: vec4<f32>, @location(3) m1: vec4<f32>, @location(4) m2: vec4<f32>, @location(5) m3: vec4<f32>,
    @location(6) color: vec4<f32>, @location(7) material: vec4<f32>
}
struct Out { @builtin(position) position: vec4<f32>, @location(0) world: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) color: vec4<f32>, @location(3) material: vec4<f32>, @location(4) light: vec4<f32> }
@vertex fn vs_main(v: In) -> Out {
    let model = mat4x4<f32>(v.m0, v.m1, v.m2, v.m3);
    let world = model * vec4<f32>(v.position, 1.0);
    var out: Out; out.position=g.view_proj*world; out.world=world.xyz;
    // Box faces are axis aligned in local space, so normalized basis transformation is exact.
    out.normal=normalize((model*vec4<f32>(v.normal,0.0)).xyz);
    out.color=v.color;out.material=v.material;out.light=g.light_vp*world;return out;
}
@vertex fn vs_shadow(v: In) -> @builtin(position) vec4<f32> {
    return g.light_vp*mat4x4<f32>(v.m0,v.m1,v.m2,v.m3)*vec4<f32>(v.position,1.0);
}
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
@fragment fn fs_main(v: Out) -> @location(0) vec4<f32> {
    let n=normalize(v.normal);let ld=normalize(vec3<f32>(-0.45,0.85,0.35));
    let shadow=scene_shadow(v.light);
    let diffuse=max(dot(n,ld),0.0);
    let ambient=mix(vec3<f32>(0.11,0.16,0.19),vec3<f32>(0.37,0.48,0.53),n.y*0.5+0.5)*(vec3<f32>(0.85)+g.accent.rgb*0.15);
    var base=v.color.rgb;
    let emission=v.material.x;
    let grid_width=max(fwidth(v.world.xz/2.0),vec2<f32>(0.001));
    if v.material.y>0.5 {
        let p=v.world.xz;let grid=abs(fract(p/2.0-vec2<f32>(0.5))-vec2<f32>(0.5))/grid_width;
        let line=1.0-min(min(grid.x,grid.y),1.0);
        base=mix(base,vec3<f32>(0.22,0.32,0.34),line*0.38);
    }
    let view=normalize(g.eye_time.xyz-v.world);let halfv=normalize(ld+view);
    let spec=pow(max(dot(n,halfv),0.0),48.0)*0.17*shadow;
    var lit=base*(ambient+vec3<f32>(1.12,1.0,0.82)*diffuse*shadow)+vec3<f32>(spec)+base*emission;
    lit+=base*axiom_lighting(v.world,n);
    let distance=length(g.eye_time.xyz-v.world);
    let fog=1.0-exp(-distance*distance*0.00012);
    lit=mix(lit,vec3<f32>(0.055,0.09,0.12),fog);
    lit=lit/(lit+vec3<f32>(0.7));
    return vec4<f32>(lit,1.0);
}
@vertex fn vs_sky(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
    let p=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));return vec4<f32>(p[i],0.9999,1.0);
}
@fragment fn fs_sky(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32> {
    let horizon=clamp(p.y/1000.0,0.0,1.0);
    return vec4<f32>(mix(vec3<f32>(0.018,0.032,0.055),vec3<f32>(0.12,0.18,0.21),horizon),1.0);
}

fn axiom_lighting(p:vec3<f32>,n:vec3<f32>)->vec3<f32> {
 var color=vec3<f32>(0.0);for(var i=0u;i<min(u32(g.accent.w),32u);i=i+1u) {
  let light=g.lights[i];let d=light.position_radius.xyz-p;let distance=length(d);let falloff=pow(max(1.0-distance/light.position_radius.w,0.0),2.0);
  color+=light.color_intensity.rgb*light.color_intensity.w*falloff*max(dot(n,normalize(d+vec3<f32>(0.00001))),0.0);
 }return color;
}
