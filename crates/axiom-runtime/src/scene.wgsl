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
@fragment fn fs_main(v: Out) -> @location(0) vec4<f32> {
    let n=normalize(v.normal);
    let shadow=sun_shadow(v.light);
    let ambient=axiom_ambient(n);
    var base=v.color.rgb;
    let emission=v.material.x;
    let grid_width=max(fwidth(v.world.xz/2.0),vec2<f32>(0.001));
    if v.material.y>0.5 {
        let p=v.world.xz;let grid=abs(fract(p/2.0-vec2<f32>(0.5))-vec2<f32>(0.5))/grid_width;
        let line=1.0-min(min(grid.x,grid.y),1.0);
        base=mix(base,vec3<f32>(0.22,0.32,0.34),line*0.38);
    }
    let view=normalize(g.eye_time.xyz-v.world);
    var direct=vec3<f32>(0.0);var spec=vec3<f32>(0.0);
    for(var i=0u;i<min(u32(g.ambient_sky.w),4u);i=i+1u) {
        let sun=g.suns[i];let ld=sun.direction_shadow.xyz;
        let lit_by=select(1.0,shadow,sun.direction_shadow.w>0.5);
        direct+=sun.color.rgb*max(dot(n,ld),0.0)*lit_by;
        spec+=sun.specular.rgb*(pow(max(dot(n,normalize(ld+view)),0.0),48.0)*0.17*lit_by);
    }
    // material.zw = render.surface (metallic, smoothness + 1; 0 = no render.surface).
    var diffuse=base;var reflection=vec3<f32>(0.0);
    if v.material.w>0.5 {
        diffuse=base*(1.0-v.material.z);
        reflection=axiom_reflection(base,n,view,v.material.z,v.material.w-1.0);
    }
    var lit=diffuse*(ambient+direct)+spec+base*emission+reflection;
    lit+=diffuse*axiom_lighting(v.world,n);
    lit=axiom_fog(lit,v.world);
    lit=lit/(lit+vec3<f32>(0.7));
    return vec4<f32>(lit,1.0);
}
@vertex fn vs_sky(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
    let p=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));return vec4<f32>(p[i],0.9999,1.0);
}
@fragment fn fs_sky(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32> {
    if g.sky_zenith.w<0.5 {
        // Legacy background: a fixed screen-space gradient (pixels from the top).
        let horizon=clamp(p.y/1000.0,0.0,1.0);
        return vec4<f32>(mix(vec3<f32>(0.018,0.032,0.055),vec3<f32>(0.12,0.18,0.21),horizon),1.0);
    }
    // Authored sky: the world direction through this pixel. NDC comes from the pixel and the
    // view's viewport rectangle: a vs_sky that indexes its corner array and also passes NDC
    // as a varying crashed the NVIDIA MX150 Vulkan driver (32.0.15.6614) in pipeline creation.
    let rel=(p.xy-g.sky_viewport.xy)/max(g.sky_viewport.zw,vec2<f32>(1.0));
    let ndc=vec2<f32>(rel.x*2.0-1.0,1.0-rel.y*2.0);
    let near=g.inv_view_proj*vec4<f32>(ndc,0.0,1.0);
    let far=g.inv_view_proj*vec4<f32>(ndc,1.0,1.0);
    let d=normalize(far.xyz/far.w-near.xyz/near.w);
    return vec4<f32>(min(axiom_sky(d,true),vec3<f32>(1.0)),1.0);
}