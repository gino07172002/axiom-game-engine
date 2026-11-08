fn hash(p:vec2<f32>)->f32 {return fract(sin(dot(p,vec2<f32>(127.1,311.7)))*43758.5453);}
fn cells(uv:vec2<f32>)->f32 {let p=fract(uv);return smoothstep(0.03,0.07,min(min(p.x,1.0-p.x),min(p.y,1.0-p.y)));}
fn wood(uv:vec2<f32>)->vec3<f32> {let f=0.5+0.5*sin(uv.x*55.0+sin(uv.y*8.0)*2.0);return mix(vec3<f32>(0.12,0.035,0.012),vec3<f32>(0.7,0.32,0.08),f);}
fn sky(r:vec3<f32>,rough:f32)->vec3<f32> {let horizon=mix(vec3<f32>(0.07,0.1,0.15),vec3<f32>(0.6,0.78,1.0),r.y*0.5+0.5);let studio=pow(max(0.0,dot(r,normalize(vec3<f32>(-0.4,0.8,0.5)))),mix(128.0,2.0,rough));return horizon*0.42+vec3<f32>(1.4)*studio;}
fn brdf(base:vec3<f32>,n:vec3<f32>,v:vec3<f32>,rough:f32,metal:f32,spec:f32)->vec3<f32> {
 let l=normalize(vec3<f32>(-0.45,0.85,0.35));let h=normalize(l+v);let nv=max(dot(n,v),0.001);let nl=max(dot(n,l),0.0);let nh=max(dot(n,h),0.0);let vh=max(dot(v,h),0.0);
 let a=max(rough*rough,0.002);let a2=a*a;let d=a2/(3.14159265*pow(nh*nh*(a2-1.0)+1.0,2.0));let k=pow(rough+1.0,2.0)/8.0;
 let g=(nv/(nv*(1.0-k)+k))*(nl/(nl*(1.0-k)+k));let f0=mix(vec3<f32>(0.04*spec),base,metal);let f=f0+(vec3<f32>(1.0)-f0)*pow(1.0-vh,5.0);
 let reflected=reflect(-v,n);let env=sky(reflected,rough)*mix(f0,vec3<f32>(1.0),pow(1.0-nv,5.0))*(1.0-rough*0.5);
 return (vec3<f32>(1.0)-f)*(1.0-metal)*base*(0.12+nl*0.9)+d*g*f/(4.0*nv*max(nl,0.001))*nl*1.4+env;
}
fn surface(s:SurfaceInput)->vec4<f32> {var base=s.base_color.rgb;let mode=s.params.x;
 if mode>0.5&&mode<1.5 {base*=wood(s.uv*2.0);}else if mode>1.5 {let c=floor(s.uv*8.0);base*=select(vec3<f32>(0.08),vec3<f32>(1.0),((i32(c.x)+i32(c.y))%2)==0);}
 let lit=brdf(base,s.normal,s.view_direction,s.params.y,s.params.z,s.params.w)*s.shadow+base*axiom_local_light(s.world_position,s.normal);
 return vec4<f32>(lit/(lit+vec3<f32>(0.8)),s.base_color.a);}