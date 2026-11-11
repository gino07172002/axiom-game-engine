// params = (alpha mode, alpha cutoff, render.surface metallic, smoothness + 1; 0 = none).
struct Draw { model:mat4x4<f32>, normal:mat4x4<f32>, color:vec4<f32>, params:vec4<f32> }
@group(1) @binding(0) var<uniform> d:Draw;
@group(1) @binding(1) var base:texture_2d<f32>;
@group(1) @binding(2) var base_sampler:sampler;
struct Out { @builtin(position) clip:vec4<f32>, @location(0) normal:vec3<f32>, @location(1) uv:vec2<f32>, @location(2) world:vec3<f32> }
@vertex fn vertex_main(@location(0) position:vec3<f32>,@location(1) normal:vec3<f32>,@location(2) uv:vec2<f32>)->Out{
 let world=d.model*vec4<f32>(position,1.);
 var o:Out;o.clip=g.view_proj*world;o.normal=(d.normal*vec4<f32>(normal,0.)).xyz;o.uv=uv;o.world=world.xyz;return o;
}
// Sun depth pass (authored lighting only): meshes cast into the same map as cubes.
@vertex fn vs_mesh_shadow(@location(0) position:vec3<f32>)->@builtin(position) vec4<f32>{
 return g.light_vp*d.model*vec4<f32>(position,1.0);
}
// Sun-0 shadow at a world position; 1 without authored lighting (the legacy look has none).
fn mesh_shadow(world:vec3<f32>)->f32{
 if g.ambient_equator.w<0.5 {return 1.0;}
 return sun_shadow(g.light_vp*vec4<f32>(world,1.0));
}
@fragment fn fragment_main(v:Out,@builtin(front_facing) front:bool)->@location(0) vec4<f32>{
 let color=textureSample(base,base_sampler,v.uv)*d.color;
 if d.params.x>0.5 && color.a<max(d.params.y,0.003){discard;}
 var n=normalize(v.normal);if !front{n=-n;}
 let alpha=select(1.,color.a,d.params.x>1.5);
 if g.ambient_equator.w>0.5 {
  // Authored lighting: the same ambient + sun (sun 0 shadowed) + local + fog + tone curve as
  // cube geometry, plus the render.surface reflection.
  let shadow=mesh_shadow(v.world);
  var direct=vec3<f32>(0.0);
  for(var i=0u;i<min(u32(g.ambient_sky.w),4u);i=i+1u){
   let sun=g.suns[i];
   direct+=sun.color.rgb*max(dot(n,sun.direction_shadow.xyz),0.0)*select(1.0,shadow,i==0u);
  }
  var diffuse=color.rgb;var reflection=vec3<f32>(0.0);
  if d.params.w>0.5 {
   let view=normalize(g.eye_time.xyz-v.world);
   diffuse=color.rgb*(1.0-d.params.z);
   reflection=axiom_reflection(color.rgb,n,view,d.params.z,d.params.w-1.0);
  }
  var lit=diffuse*(axiom_ambient(n)+direct+axiom_lighting(v.world,n))+reflection;
  lit=axiom_fog(lit,v.world);
  return vec4<f32>(lit/(lit+vec3<f32>(0.7)),alpha);
 }
 // Legacy look (no authored directional light or environment): flat key light.
 let lighting=0.67+0.33*max(dot(n,normalize(vec3<f32>(-0.4,0.8,0.5))),0.);
 return vec4<f32>(color.rgb*(vec3<f32>(lighting)+axiom_lighting(v.world,n)),alpha);
}
