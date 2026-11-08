struct Volume {center_kind:vec4<f32>,half_strength:vec4<f32>,color:vec4<f32>}
struct Settings {inverse:mat4x4<f32>,eye_time:vec4<f32>,viewport:vec4<f32>,info:vec4<f32>,volumes:array<Volume,8>}
@group(0) @binding(0) var scene:texture_2d<f32>;
@group(0) @binding(1) var depth:texture_depth_2d;
@group(0) @binding(2) var linear_sampler:sampler;
@group(0) @binding(3) var<uniform> s:Settings;
@vertex fn vertex(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
 let p=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));return vec4<f32>(p[i],0.0,1.0);
}
fn position(uv:vec2<f32>,z:f32)->vec3<f32> {let p=s.inverse*vec4<f32>(uv.x*2.0-1.0,1.0-uv.y*2.0,z,1.0);return p.xyz/p.w;}
fn noise(p:vec3<f32>)->f32 {return 0.5+0.5*sin(p.x*3.1+sin(p.z*2.3))*sin(p.y*2.6+sin(p.x*1.7));}
@fragment fn fragment(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32> {
 let dimensions=vec2<f32>(textureDimensions(scene));let uv=p.xy/dimensions;let local=(p.xy-s.viewport.xy)/s.viewport.zw;
 let z=textureLoad(depth,vec2<i32>(p.xy),0);let world=position(local,z);let start=position(local,0.0);let direction=normalize(world-start);let scene_distance=length(world-start);
 var color=textureSampleLevel(scene,linear_sampler,uv,0.0).rgb;
 for(var i=0u;i<min(u32(s.info.x),8u);i=i+1u){
  let v=s.volumes[i];let low=v.center_kind.xyz-v.half_strength.xyz;let high=v.center_kind.xyz+v.half_strength.xyz;
  if v.center_kind.w>2.5 {
   if z<0.99999 && all(world>=low) && all(world<=high) {
    let q=(world.xz-v.center_kind.xz)/v.half_strength.xz;let r=length(q);
    let ring=1.0-smoothstep(0.03,0.08,abs(r-0.65));let cross=1.0-smoothstep(0.03,0.07,min(abs(q.x-q.y),abs(q.x+q.y)));
    let mask=max(ring,cross*0.55)*(1.0-smoothstep(0.8,1.0,r));
    color=mix(color,v.color.rgb,mask*v.color.a*v.half_strength.w);
   }
   continue;
  }
  let safe=select(direction,vec3<f32>(0.000001),abs(direction)<vec3<f32>(0.000001));let a=(low-start)/safe;let b=(high-start)/safe;
  let t0=max(max(max(min(a.x,b.x),min(a.y,b.y)),min(a.z,b.z)),0.0);let t1=min(min(min(max(a.x,b.x),max(a.y,b.y)),max(a.z,b.z)),scene_distance);
  if t1<=t0 {continue;}
  let weight=clamp((t1-t0)*0.6,0.0,1.0);
  if v.center_kind.w<0.5 {
   let offset=vec2<f32>(0.012*v.half_strength.w*weight,0.0);
   let aberration=vec3<f32>(textureSampleLevel(scene,linear_sampler,uv+offset,0.0).r,color.g,textureSampleLevel(scene,linear_sampler,uv-offset,0.0).b);
   color=mix(color,aberration*v.color.rgb,weight);
  }else if v.center_kind.w<1.5 {
   let offset=vec2<f32>(sin(local.y*57.0-s.eye_time.w*5.0),cos(local.x*43.0+s.eye_time.w*3.0))*0.014*v.half_strength.w*weight;
   color=textureSampleLevel(scene,linear_sampler,clamp(uv+offset,vec2<f32>(0.0),vec2<f32>(1.0)),0.0).rgb;
  }else {
   var density=0.0;for(var step=0u;step<12u;step=step+1u){let t=mix(t0,t1,(f32(step)+0.5)/12.0);density+=noise((start+direction*t)*1.3+vec3<f32>(0.0,-s.eye_time.w*0.18,0.0));}
   let opacity=1.0-exp(-density/12.0*(t1-t0)*v.half_strength.w);
   color=mix(color,v.color.rgb,opacity*v.color.a);
  }
 }
 return vec4<f32>(color,1.0);
}
