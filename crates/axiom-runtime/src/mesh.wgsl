struct LocalLight {position_radius:vec4<f32>,color_intensity:vec4<f32>}
struct Globals { view_proj: mat4x4<f32>, light_vp: mat4x4<f32>, eye_time: vec4<f32>, accent: vec4<f32>, lights:array<LocalLight,32>, view_params:vec4<f32> }
struct Draw { model:mat4x4<f32>, normal:mat4x4<f32>, color:vec4<f32>, params:vec4<f32> }
@group(0) @binding(0) var<uniform> g:Globals;
@group(1) @binding(0) var<uniform> d:Draw;
@group(1) @binding(1) var base:texture_2d<f32>;
@group(1) @binding(2) var base_sampler:sampler;
struct Out { @builtin(position) clip:vec4<f32>, @location(0) normal:vec3<f32>, @location(1) uv:vec2<f32>, @location(2) world:vec3<f32> }
@vertex fn vertex_main(@location(0) position:vec3<f32>,@location(1) normal:vec3<f32>,@location(2) uv:vec2<f32>)->Out{
 let world=d.model*vec4<f32>(position,1.);
 var o:Out;o.clip=g.view_proj*world;o.normal=(d.normal*vec4<f32>(normal,0.)).xyz;o.uv=uv;o.world=world.xyz;return o;
}
@fragment fn fragment_main(v:Out,@builtin(front_facing) front:bool)->@location(0) vec4<f32>{
 let color=textureSample(base,base_sampler,v.uv)*d.color;
 if d.params.x>0.5 && color.a<max(d.params.y,0.003){discard;}
 var n=normalize(v.normal);if !front{n=-n;}
 let lighting=0.67+0.33*max(dot(n,normalize(vec3<f32>(-0.4,0.8,0.5))),0.);
 return vec4<f32>(color.rgb*(vec3<f32>(lighting)+axiom_lighting(v.world,n)),select(1.,color.a,d.params.x>1.5));
}

fn axiom_lighting(p:vec3<f32>,n:vec3<f32>)->vec3<f32> {
 var color=vec3<f32>(0.0);for(var i=0u;i<min(u32(g.accent.w),32u);i=i+1u) {
  let light=g.lights[i];let d=light.position_radius.xyz-p;let distance=length(d);let falloff=pow(max(1.0-distance/light.position_radius.w,0.0),2.0);
  color+=light.color_intensity.rgb*light.color_intensity.w*falloff*max(dot(n,normalize(d+vec3<f32>(0.00001))),0.0);
 }return color;
}
