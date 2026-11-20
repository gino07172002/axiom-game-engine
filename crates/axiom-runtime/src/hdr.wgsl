// HDR finish (hdr.rs): bloom mip chain (Jimenez 2014 13-tap downsample with a Karis average and
// soft-knee threshold on the first level, 3x3 tent upsample) and exposure + ACES tone mapping.
// Every pass covers one camera view: the viewport is the view rectangle at the target's scale and
// samples are clamped inside the view rectangle, so a view never blooms into its neighbour.
struct Finish {
 // View rectangle in normalized frame coordinates (x, y, width, height).
 rect: vec4<f32>,
 // threshold, soft knee, scatter (bloom radius), intensity / sum of the level weights.
 bloom: vec4<f32>,
 // exposure, 1 = add bloom, 0, 0.
 tone: vec4<f32>,
}
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var linear_clamp: sampler;
@group(0) @binding(2) var<uniform> f: Finish;
@group(0) @binding(3) var glow: texture_2d<f32>;

struct Varying { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn vertex(@builtin(vertex_index) i: u32) -> Varying {
 let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
 var o: Varying;
 o.position = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
 // Top of the viewport = uv.y 0, like framebuffer pixels.
 o.uv = f.rect.xy + vec2<f32>(p.x, 1.0 - p.y) * f.rect.zw;
 return o;
}

fn tap(uv: vec2<f32>, texel: vec2<f32>) -> vec3<f32> {
 let lo = f.rect.xy + texel * 0.5;
 let hi = f.rect.xy + f.rect.zw - texel * 0.5;
 return textureSampleLevel(source, linear_clamp, clamp(uv, lo, hi), 0.0).rgb;
}
fn luma(c: vec3<f32>) -> f32 { return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)); }
fn karis(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
 let s = (a + b + c + d) * 0.25;
 return s / (1.0 + luma(s));
}

@fragment fn prefilter(v: Varying) -> @location(0) vec4<f32> {
 let t = 1.0 / vec2<f32>(textureDimensions(source));
 let a = tap(v.uv + t * vec2<f32>(-2.0, -2.0), t); let b = tap(v.uv + t * vec2<f32>(0.0, -2.0), t);
 let c = tap(v.uv + t * vec2<f32>(2.0, -2.0), t); let d = tap(v.uv + t * vec2<f32>(-2.0, 0.0), t);
 let e = tap(v.uv, t); let ff = tap(v.uv + t * vec2<f32>(2.0, 0.0), t);
 let g = tap(v.uv + t * vec2<f32>(-2.0, 2.0), t); let h = tap(v.uv + t * vec2<f32>(0.0, 2.0), t);
 let i = tap(v.uv + t * vec2<f32>(2.0, 2.0), t); let j = tap(v.uv + t * vec2<f32>(-1.0, -1.0), t);
 let k = tap(v.uv + t * vec2<f32>(1.0, -1.0), t); let l = tap(v.uv + t * vec2<f32>(-1.0, 1.0), t);
 let m = tap(v.uv + t * vec2<f32>(1.0, 1.0), t);
 // Karis average of the five 2x2 groups: one very bright texel cannot flicker the whole glow.
 var c0 = karis(j, k, l, m) * 0.5;
 c0 += karis(a, b, d, e) * 0.125 + karis(b, c, e, ff) * 0.125;
 c0 += karis(d, e, g, h) * 0.125 + karis(e, ff, h, i) * 0.125;
 // Karis weights divided by 1 + luma; undo it on the result so the threshold sees radiance.
 c0 = c0 / max(1.0 - luma(c0), 0.0001);
 c0 = min(c0, vec3<f32>(1024.0));
 let brightness = max(c0.r, max(c0.g, c0.b));
 let knee = f.bloom.y;
 var soft = clamp(brightness - f.bloom.x + knee, 0.0, 2.0 * knee);
 soft = soft * soft / (4.0 * knee + 0.00001);
 let contribution = max(soft, brightness - f.bloom.x) / max(brightness, 0.00001);
 return vec4<f32>(c0 * contribution, 1.0);
}

@fragment fn downsample(v: Varying) -> @location(0) vec4<f32> {
 let t = 1.0 / vec2<f32>(textureDimensions(source));
 let a = tap(v.uv + t * vec2<f32>(-2.0, -2.0), t); let b = tap(v.uv + t * vec2<f32>(0.0, -2.0), t);
 let c = tap(v.uv + t * vec2<f32>(2.0, -2.0), t); let d = tap(v.uv + t * vec2<f32>(-2.0, 0.0), t);
 let e = tap(v.uv, t); let ff = tap(v.uv + t * vec2<f32>(2.0, 0.0), t);
 let g = tap(v.uv + t * vec2<f32>(-2.0, 2.0), t); let h = tap(v.uv + t * vec2<f32>(0.0, 2.0), t);
 let i = tap(v.uv + t * vec2<f32>(2.0, 2.0), t); let j = tap(v.uv + t * vec2<f32>(-1.0, -1.0), t);
 let k = tap(v.uv + t * vec2<f32>(1.0, -1.0), t); let l = tap(v.uv + t * vec2<f32>(-1.0, 1.0), t);
 let m = tap(v.uv + t * vec2<f32>(1.0, 1.0), t);
 let o = e * 0.125 + (a + c + g + i) * 0.03125 + (b + d + ff + h) * 0.0625 + (j + k + l + m) * 0.125;
 return vec4<f32>(o, 1.0);
}

fn tent(uv: vec2<f32>, t: vec2<f32>) -> vec3<f32> {
 var s = tap(uv, t) * 4.0;
 s += (tap(uv + vec2<f32>(t.x, 0.0), t) + tap(uv - vec2<f32>(t.x, 0.0), t)
  + tap(uv + vec2<f32>(0.0, t.y), t) + tap(uv - vec2<f32>(0.0, t.y), t)) * 2.0;
 s += tap(uv + t, t) + tap(uv - t, t) + tap(uv + vec2<f32>(t.x, -t.y), t) + tap(uv + vec2<f32>(-t.x, t.y), t);
 return s / 16.0;
}

// Added (blend One + One) onto the next finer level: level k ends up weighted scatter^k.
@fragment fn upsample(v: Varying) -> @location(0) vec4<f32> {
 let t = 1.0 / vec2<f32>(textureDimensions(source));
 return vec4<f32>(tent(v.uv, t) * f.bloom.z, 1.0);
}

// Hill's fit of the ACES RRT + sRGB ODT (BakingLab ACESFitted); input and output linear sRGB.
fn rrt_odt(v: vec3<f32>) -> vec3<f32> {
 let a = v * (v + 0.0245786) - 0.000090537;
 let b = v * (0.983729 * v + 0.4329510) + 0.238081;
 return a / b;
}
fn aces_fitted(c: vec3<f32>) -> vec3<f32> {
 let i = vec3<f32>(dot(vec3<f32>(0.59719, 0.35458, 0.04823), c),
  dot(vec3<f32>(0.07600, 0.90834, 0.01566), c), dot(vec3<f32>(0.02840, 0.13383, 0.83777), c));
 let r = rrt_odt(i);
 let o = vec3<f32>(dot(vec3<f32>(1.60475, -0.53108, -0.07367), r),
  dot(vec3<f32>(-0.10208, 1.10813, -0.00605), r), dot(vec3<f32>(-0.00327, -0.07276, 1.07602), r));
 return clamp(o, vec3<f32>(0.0), vec3<f32>(1.0));
}

// The scene at this pixel (same size as the target) plus the bloom, exposed and tone mapped;
// the sRGB target encodes the result.
@fragment fn tonemap(v: Varying) -> @location(0) vec4<f32> {
 var c = textureLoad(source, vec2<i32>(v.position.xy), 0).rgb;
 // Guard against f16 infinity / NaN from extreme authored values.
 c = clamp(c, vec3<f32>(0.0), vec3<f32>(60000.0));
 if f.tone.y > 0.5 {
  let t = 1.0 / vec2<f32>(textureDimensions(glow));
  let lo = f.rect.xy + t * 0.5;
  let hi = f.rect.xy + f.rect.zw - t * 0.5;
  c += textureSampleLevel(glow, linear_clamp, clamp(v.uv, lo, hi), 0.0).rgb * f.bloom.w;
 }
 return vec4<f32>(aces_fitted(c * f.tone.x), 1.0);
}
