// params = (alpha mode, alpha cutoff, render.surface metallic, smoothness + 1; 0 = none).
// Stylized look (mesh_style.rs; all zero = none): toon = (bands, 0 = no toon; softness;
// threshold; skin wrap), style = (rim power, face blend, outline width: > 0 m, < 0 px;
// decal opacity), face = (face direction xyz in world; decal mode: 0 none, 1 alpha,
// 2 multiply, 3 add, + 4 projected), decal_u/v/w = rows giving (image u, image v, box depth)
// from (uv, 0, 1) or (world, 1), colors = RGBA8 shadow, rim (a = strength), skin tint, outline.
struct Draw { model:mat4x4<f32>, normal:mat4x4<f32>, color:vec4<f32>, params:vec4<f32>, toon:vec4<f32>, style:vec4<f32>, face:vec4<f32>, decal_u:vec4<f32>, decal_v:vec4<f32>, decal_w:vec4<f32>, colors:vec4<u32> }
@group(1) @binding(0) var<uniform> d:Draw;
@group(1) @binding(1) var base:texture_2d<f32>;
@group(1) @binding(2) var base_sampler:sampler;
@group(1) @binding(3) var decal_map:texture_2d<f32>;
@group(1) @binding(4) var decal_sampler:sampler;
struct Out { @builtin(position) clip:vec4<f32>, @location(0) normal:vec3<f32>, @location(1) uv:vec2<f32>, @location(2) world:vec3<f32> }
@vertex fn vertex_main(@location(0) position:vec3<f32>,@location(1) normal:vec3<f32>,@location(2) uv:vec2<f32>)->Out{
 let world=d.model*vec4<f32>(position,1.);
 var o:Out;o.clip=g.view_proj*world;o.normal=(d.normal*vec4<f32>(normal,0.)).xyz;o.uv=uv;o.world=world.xyz;return o;
}
// Sun depth pass (authored lighting only): meshes cast into the same map as cubes.
@vertex fn vs_mesh_shadow(@location(0) position:vec3<f32>)->@builtin(position) vec4<f32>{
 return g.light_vp*d.model*vec4<f32>(position,1.0);
}
// render.outline inverted hull: front faces culled, pushed out along the world normal. Pixel
// widths become meters at the vertex's depth (a pixel spans 2*w/(height*P11) world units; P11 is
// the length of view_proj's second row, the view rotation being orthonormal), so camera-facing
// vertices move toward the camera like the meter mode instead of sliding sideways on screen
// (which flips small triangles' winding and lets the hull cover the surface). Position only.
@vertex fn vs_outline(@location(0) position:vec3<f32>,@location(1) normal:vec3<f32>)->@builtin(position) vec4<f32>{
 let p=(d.model*vec4<f32>(position,1.0)).xyz;
 let m=(d.normal*vec4<f32>(normal,0.0)).xyz;
 let nl=length(m);
 var n=vec3<f32>(0.0);
 if nl>1e-8 {n=m/nl;}
 var width=d.style.z;
 if width<0.0 {
  let p11=max(length(vec3<f32>(g.view_proj[0].y,g.view_proj[1].y,g.view_proj[2].y)),1e-6);
  let w=(g.view_proj*vec4<f32>(p,1.0)).w;
  width=-width*2.0*w/(max(g.sky_viewport.w,1.0)*p11);
 }
 return g.view_proj*vec4<f32>(p+n*width,1.0);
}@fragment fn fs_outline()->@location(0) vec4<f32>{
 return vec4<f32>(unpack4x8unorm(d.colors.w).rgb,1.0);
}
// Sun-0 shadow at a world position; 1 without authored lighting (the legacy look has none).
fn mesh_shadow(world:vec3<f32>)->f32{
 if g.ambient_equator.w<0.5 {return 1.0;}
 return sun_shadow(g.light_vp*vec4<f32>(world,1.0));
}
// render.decal over a base colour (alpha unchanged). `n` is the shaded side's normal.
fn mesh_decal(color:vec4<f32>,uv:vec2<f32>,world:vec3<f32>,n:vec3<f32>)->vec4<f32>{
 var mode=d.face.w;
 if mode<0.5 {return color;}
 var p=vec4<f32>(uv,0.0,1.0);
 if mode>3.5 {
  if dot(n,d.decal_w.xyz)<=0.0 {return color;}
  p=vec4<f32>(world,1.0);mode=mode-4.0;
 }
 let t=vec3<f32>(dot(d.decal_u,p),dot(d.decal_v,p),dot(d.decal_w,p));
 if any(t.xy<vec2<f32>(0.0)) || any(t.xy>vec2<f32>(1.0)) || abs(t.z)>0.5 {return color;}
 let s=textureSampleLevel(decal_map,decal_sampler,t.xy,0.0);
 let a=s.a*d.style.w;
 if mode<1.5 {return vec4<f32>(mix(color.rgb,s.rgb,a),color.a);}
 if mode<2.5 {return vec4<f32>(mix(color.rgb,color.rgb*s.rgb,a),color.a);}
 return vec4<f32>(color.rgb+s.rgb*a,color.a);
}
// Light level of the toon ramp for a light cosine: 0 on the shadow side, then `bands` steps to 1.
// The skin wrap lets light reach past the terminator (wrap lighting).
fn toon_ramp(ndl:f32)->f32{
 let bands=d.toon.x;
 let x=((ndl+d.toon.w)/(1.0+d.toon.w)-d.toon.z)/(1.0-d.toon.z);
 let w=max(d.toon.y*0.5/bands,0.0005);
 var level=0.0;
 for(var k=0u;k<8u;k=k+1u){
  if f32(k)>=bands {break;}
  let edge=f32(k)/bands;
  level+=smoothstep(edge-w,edge+w,x);
 }
 return level/bands;
}
// Skin APPROXIMATION (not subsurface scattering): a band around the terminator, tinted.
fn toon_terminator(ndl:f32)->f32{
 let x=(ndl+d.toon.w)/(1.0+d.toon.w)-d.toon.z;
 return d.toon.w*(1.0-smoothstep(0.0,0.35,abs(x)));
}
// render.toon: banded sun light with tinted shadows, rim, optional face direction and skin
// approximation; the same ambient, local lights, fog and tone curve as the lit path.
fn toon_fragment(color:vec4<f32>,n:vec3<f32>,world:vec3<f32>,alpha:f32)->vec4<f32>{
 let view=normalize(g.eye_time.xyz-world);
 let bent=mix(n,d.face.xyz,d.style.y);
 let ns=select(n,normalize(bent),length(bent)>1e-4);
 let shadow_tint=unpack4x8unorm(d.colors.x).rgb;
 let rim=unpack4x8unorm(d.colors.y);
 let skin=unpack4x8unorm(d.colors.z).rgb;
 let authored=g.ambient_equator.w>0.5;
 var ambient=vec3<f32>(0.67);
 var key=vec3<f32>(0.0);var lit0=0.0;var term=0.0;var sun0=vec3<f32>(0.33);
 if authored {
  ambient=axiom_ambient(ns);
  let shadow=smoothstep(0.35,0.65,mesh_shadow(world));
  for(var i=0u;i<min(u32(g.ambient_sky.w),4u);i=i+1u){
   let sun=g.suns[i];
   let ndl=dot(ns,sun.direction_shadow.xyz);
   let r=toon_ramp(ndl)*select(1.0,shadow,i==0u);
   key+=sun.color.rgb*r;
   if i==0u {lit0=r;term=toon_terminator(ndl)*shadow;sun0=sun.color.rgb;}
  }
 } else {
  // Legacy look: its fixed key light, banded.
  let ndl=dot(ns,normalize(vec3<f32>(-0.4,0.8,0.5)));
  lit0=toon_ramp(ndl);term=toon_terminator(ndl);key=vec3<f32>(0.33*lit0);
 }
 let shade=mix(shadow_tint,vec3<f32>(1.0),lit0);
 var lit=color.rgb*(ambient*shade+key+skin*term*sun0+axiom_lighting(world,ns));
 lit+=rim.rgb*rim.a*pow(1.0-clamp(dot(n,view),0.0,1.0),d.style.x);
 if !authored {return vec4<f32>(lit,alpha);}
 lit=axiom_fog(lit,world);
 return vec4<f32>(lit/(lit+vec3<f32>(0.7)),alpha);
}
@fragment fn fragment_main(v:Out,@builtin(front_facing) front:bool)->@location(0) vec4<f32>{
 var color=textureSample(base,base_sampler,v.uv)*d.color;
 if d.params.x>0.5 && color.a<max(d.params.y,0.003){discard;}
 var n=normalize(v.normal);if !front{n=-n;}
 color=mesh_decal(color,v.uv,v.world,n);
 let alpha=select(1.,color.a,d.params.x>1.5);
 // view_params.w = 1: Unlit display (flat base colour: no toon look, no reflections).
 if d.toon.x>0.5 && g.view_params.w<0.5 {return toon_fragment(color,n,v.world,alpha);}
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
  if d.params.w>0.5 && g.view_params.w<0.5 {
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
