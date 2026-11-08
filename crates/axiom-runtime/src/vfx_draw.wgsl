struct Light {position_radius:vec4<f32>,color_intensity:vec4<f32>}
struct Globals {view_proj:mat4x4<f32>,light_vp:mat4x4<f32>,eye_time:vec4<f32>,accent:vec4<f32>,lights:array<Light,32>,view_params:vec4<f32>}
@group(0) @binding(0) var<uniform> g:Globals;
@group(0) @binding(1) var scene_depth:texture_depth_2d;
struct Out {@builtin(position) clip:vec4<f32>,@location(0) uv:vec2<f32>,@location(1) color:vec4<f32>,@location(2) kind:f32}
fn point(p:vec4<f32>,color:vec4<f32>,misc:vec4<f32>,i:u32)->Out {
 let corners=array<vec2<f32>,6>(vec2<f32>(-1.0,-1.0),vec2<f32>(1.0,-1.0),vec2<f32>(1.0,1.0),vec2<f32>(-1.0,-1.0),vec2<f32>(1.0,1.0),vec2<f32>(-1.0,1.0));
 let uv=corners[i];let c=cos(misc.x);let s=sin(misc.x);let rotated=vec2<f32>(uv.x*c-uv.y*s,uv.x*s+uv.y*c);
 let forward=normalize(g.eye_time.xyz-p.xyz+vec3<f32>(0.0,0.0,0.000001));let right=normalize(cross(vec3<f32>(0.0,1.0,0.0),forward)+vec3<f32>(0.000001,0.0,0.0));let up=cross(forward,right);
 var o:Out;o.clip=g.view_proj*vec4<f32>(p.xyz+(right*rotated.x+up*rotated.y)*p.w,1.0);o.uv=uv;o.color=color;o.kind=0.0;return o;
}
@vertex fn sprite(@location(0) p:vec4<f32>,@location(1) color:vec4<f32>,@location(2) misc:vec4<f32>,@builtin(vertex_index) i:u32)->Out {return point(p,color,misc,i);}
@vertex fn mesh(@location(0) p:vec4<f32>,@location(1) color:vec4<f32>,@location(2) misc:vec4<f32>,@builtin(vertex_index) i:u32)->Out {
 let pos=array<vec3<f32>,8>(vec3<f32>(-1.0,-1.0,-1.0),vec3<f32>(1.0,-1.0,-1.0),vec3<f32>(1.0,1.0,-1.0),vec3<f32>(-1.0,1.0,-1.0),vec3<f32>(-1.0,-1.0,1.0),vec3<f32>(1.0,-1.0,1.0),vec3<f32>(1.0,1.0,1.0),vec3<f32>(-1.0,1.0,1.0));
 let indices=array<u32,36>(0u,2u,1u,0u,3u,2u,4u,5u,6u,4u,6u,7u,0u,1u,5u,0u,5u,4u,3u,7u,6u,3u,6u,2u,0u,4u,7u,0u,7u,3u,1u,2u,6u,1u,6u,5u);
 let v=pos[indices[i]]*p.w;let c=cos(misc.x);let s=sin(misc.x);let world=p.xyz+vec3<f32>(v.x*c-v.z*s,v.y,v.x*s+v.z*c);
 var o:Out;o.clip=g.view_proj*vec4<f32>(world,1.0);o.uv=vec2<f32>(0.0);o.color=vec4<f32>(color.rgb*(0.5+0.5*f32((i/6u)%3u)/2.0),color.a);o.kind=1.0;return o;
}
@vertex fn ribbon(@location(0) p:vec4<f32>,@location(1) tangent_side:vec4<f32>,@location(2) color:vec4<f32>)->Out {
 let side=normalize(cross(normalize(tangent_side.xyz+vec3<f32>(0.0001)),normalize(g.eye_time.xyz-p.xyz+vec3<f32>(0.0,0.0,0.000001)))+vec3<f32>(0.000001));
 var o:Out;o.clip=g.view_proj*vec4<f32>(p.xyz+side*p.w*tangent_side.w,1.0);o.uv=vec2<f32>(tangent_side.w,0.0);o.color=color;o.kind=2.0;return o;
}
fn linear_depth(d:f32)->f32 {if g.view_params.z>0.5 {return mix(g.view_params.x,g.view_params.y,d);}return g.view_params.x*g.view_params.y/(g.view_params.y-d*(g.view_params.y-g.view_params.x));}
@fragment fn fragment(p:Out)->@location(0) vec4<f32> {
 var alpha=p.color.a;
 if p.kind<0.5 {let r=length(p.uv);alpha*=exp(-r*r*2.5)*(1.0-smoothstep(0.65,1.0,r));}
 if p.kind>1.5 {alpha*=1.0-abs(p.uv.x)*0.4;}
 let z=textureLoad(scene_depth,vec2<i32>(p.clip.xy),0);let soft=clamp((linear_depth(z)-linear_depth(p.clip.z))/0.3,0.0,1.0);
 return vec4<f32>(p.color.rgb,alpha*soft);
}
