// Shared per-view globals and lighting (prepended to scene, mesh and VFX shaders).
// Local light kinds (direction_kind.w): 0 point, 1 spot, 2 area rect, 3 area disk.
// params: spot = (cos inner half-angle, cos outer half-angle); area = (half width/radius, half height).
// axis = (area right axis xyz, point/spot source radius).
struct LocalLight {position_range:vec4<f32>,color_intensity:vec4<f32>,direction_kind:vec4<f32>,params:vec4<f32>,axis:vec4<f32>}
// direction_shadow.xyz points TOWARD the light; w = 1 when this sun samples the shadow map.
struct SunLight {direction_shadow:vec4<f32>,color:vec4<f32>,specular:vec4<f32>}
// accent.w = local light count; ambient_sky.w = sun count; ambient_equator.w = 1 when the world
// authors lighting (directional light or render.environment); fog = (rgb, k) with 1-exp(-d²k).
struct Globals { view_proj: mat4x4<f32>, light_vp: mat4x4<f32>, eye_time: vec4<f32>, accent: vec4<f32>, lights:array<LocalLight,32>, view_params:vec4<f32>, suns:array<SunLight,4>, ambient_sky:vec4<f32>, ambient_equator:vec4<f32>, ambient_ground:vec4<f32>, fog:vec4<f32> }
@group(0) @binding(0) var<uniform> g: Globals;

// Hemisphere ("trilight") ambient: equator at the horizon, sky up, ground down.
fn axiom_ambient(n:vec3<f32>)->vec3<f32> {
 let t=clamp(n.y,-1.0,1.0);
 if t>=0.0 {return mix(g.ambient_equator.rgb,g.ambient_sky.rgb,t);}
 return mix(g.ambient_equator.rgb,g.ambient_ground.rgb,-t);
}
fn axiom_sun_diffuse(n:vec3<f32>)->vec3<f32> {
 var c=vec3<f32>(0.0);for(var i=0u;i<min(u32(g.ambient_sky.w),4u);i=i+1u) {c+=g.suns[i].color.rgb*max(dot(n,g.suns[i].direction_shadow.xyz),0.0);}
 return c;
}
fn axiom_fog(lit:vec3<f32>,p:vec3<f32>)->vec3<f32> {
 let distance=length(g.eye_time.xyz-p);
 return mix(lit,g.fog.rgb,1.0-exp(-distance*distance*g.fog.w));
}
// Point, spot and (approximate) area lights. Area lights use the point of the rect/disk
// closest to the shaded point plus a one-sided emitter cosine; this is not LTC-accurate.
fn axiom_lighting(p:vec3<f32>,n:vec3<f32>)->vec3<f32> {
 var color=vec3<f32>(0.0);for(var i=0u;i<min(u32(g.accent.w),32u);i=i+1u) {
  let light=g.lights[i];let kind=light.direction_kind.w;let axis=light.direction_kind.xyz;
  var at=light.position_range.xyz;
  if kind>1.5 {
   let local=p-at;let right=light.axis.xyz;
   if kind<2.5 {
    let up=cross(right,axis);
    at+=right*clamp(dot(local,right),-light.params.z,light.params.z)+up*clamp(dot(local,up),-light.params.w,light.params.w);
   } else {
    let planar=local-axis*dot(local,axis);at+=planar*min(1.0,light.params.z/max(length(planar),0.00001));
   }
  }
  let d=at-p;let distance=length(d);let falloff=pow(max(1.0-distance/light.position_range.w,0.0),2.0);
  let l=normalize(d+vec3<f32>(0.00001));
  let wrap=clamp(light.axis.w/max(distance,0.0001),0.0,1.0)*select(1.0,0.0,kind>1.5);
  let ndl=max((dot(n,l)+wrap)/(1.0+wrap),0.0);
  var cone=1.0;
  if kind>0.5&&kind<1.5 {cone=smoothstep(light.params.y,light.params.x,dot(-l,axis));}
  if kind>1.5 {cone=max(dot(-l,axis),0.0);}
  color+=light.color_intensity.rgb*light.color_intensity.w*falloff*ndl*cone;
 }return color;
}
