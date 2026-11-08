struct Emitter {origin_time:vec4<f32>,velocity_life:vec4<f32>,accel_spread:vec4<f32>,color_a:vec4<f32>,color_b:vec4<f32>,size_misc:vec4<f32>,ids:vec4<u32>}
struct Particle {position_size:vec4<f32>,color:vec4<f32>,misc:vec4<f32>}
@group(0) @binding(0) var<uniform> e:Emitter;
@group(1) @binding(0) var<storage,read_write> particles:array<Particle>;
fn random(index:u32,salt:u32)->f32 {
 var v=e.ids.y+index*747796405u+salt*2891336453u;
 v=((v>>((v>>28u)+4u))^v)*277803737u;v=(v>>22u)^v;
 return f32(v&0x00ffffffu)/16777216.0;
}
@compute @workgroup_size(64) fn simulate(@builtin(global_invocation_id) id:vec3<u32>) {
 let i=id.x;if i>=e.ids.x {return;}
 let age=fract(e.origin_time.w/e.velocity_life.w+random(i,0u))*e.velocity_life.w;
 let t=age/e.velocity_life.w;
 var spawn=vec3<f32>((random(i,1u)-0.5)*e.accel_spread.w,0.0,(random(i,2u)-0.5)*e.accel_spread.w);
 var velocity=e.velocity_life.xyz+vec3<f32>((random(i,3u)-0.5)*e.accel_spread.w,random(i,4u)*e.accel_spread.w,(random(i,5u)-0.5)*e.accel_spread.w);
 if e.ids.w==2u {spawn=vec3<f32>((random(i,1u)-0.5)*3.5,3.5,(random(i,2u)-0.5)*3.5);velocity.y=-abs(velocity.y);}
 var p=e.origin_time.xyz+spawn+velocity*age+e.accel_spread.xyz*age*age*0.5;
 if e.ids.w==1u {p.x+=sin(age*2.1+random(i,5u)*6.0)*age*0.16;p.z+=cos(age*1.7+random(i,2u)*6.0)*age*0.16;}
 particles[e.ids.z+i]=Particle(vec4<f32>(p,mix(e.size_misc.x,e.size_misc.y,t)),mix(e.color_a,e.color_b,t),vec4<f32>(random(i,6u)*6.28318530718+age*0.6,e.size_misc.z,t,0.0));
}
