// FXAA (hdr.rs): a post-process edge filter after Lottes' FXAA 3.11 quality preset (luma edge
// detection, 12-step search along the edge, sub-pixel blend). It filters the final low dynamic
// range colours of the whole frame; it is not multisampling.
@group(0) @binding(0) var frame: texture_2d<f32>;
@group(0) @binding(1) var linear_clamp: sampler;

const EDGE_MIN: f32 = 0.0312;
const EDGE_MAX: f32 = 0.125;
const SUBPIXEL: f32 = 0.75;
const STEPS: u32 = 12u;

@vertex fn vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
 let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
 return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

// The texture is sRGB: samples are linear, the edge test uses perceptual (gamma ~2) luma.
fn luma(c: vec3<f32>) -> f32 { return sqrt(dot(c, vec3<f32>(0.299, 0.587, 0.114))); }
fn at(uv: vec2<f32>) -> vec3<f32> { return textureSampleLevel(frame, linear_clamp, uv, 0.0).rgb; }
fn l(uv: vec2<f32>) -> f32 { return luma(at(uv)); }
fn quality(i: u32) -> f32 {
 if i < 5u { return 1.0; }
 if i == 5u { return 1.5; }
 if i < 10u { return 2.0; }
 if i == 10u { return 4.0; }
 return 8.0;
}

@fragment fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
 let inv = 1.0 / vec2<f32>(textureDimensions(frame));
 let uv = position.xy * inv;
 let center = at(uv);
 let m = luma(center);
 let n = l(uv + vec2<f32>(0.0, -inv.y)); let s = l(uv + vec2<f32>(0.0, inv.y));
 let e = l(uv + vec2<f32>(inv.x, 0.0)); let w = l(uv + vec2<f32>(-inv.x, 0.0));
 let lo = min(m, min(min(n, s), min(e, w)));
 let hi = max(m, max(max(n, s), max(e, w)));
 let range = hi - lo;
 if range < max(EDGE_MIN, hi * EDGE_MAX) { return vec4<f32>(center, 1.0); }
 let nw = l(uv - inv); let se = l(uv + inv);
 let ne = l(uv + vec2<f32>(inv.x, -inv.y)); let sw = l(uv + vec2<f32>(-inv.x, inv.y));
 let edge_h = abs(-2.0 * w + nw + sw) + 2.0 * abs(-2.0 * m + n + s) + abs(-2.0 * e + ne + se);
 let edge_v = abs(-2.0 * n + nw + ne) + 2.0 * abs(-2.0 * m + w + e) + abs(-2.0 * s + sw + se);
 let horizontal = edge_h >= edge_v;
 // Side 1 = the negative direction (north / west), side 2 = south / east.
 let l1 = select(w, n, horizontal);
 let l2 = select(e, s, horizontal);
 let g1 = l1 - m; let g2 = l2 - m;
 let steep1 = abs(g1) >= abs(g2);
 let scaled = 0.25 * max(abs(g1), abs(g2));
 var stride = select(inv.x, inv.y, horizontal);
 var local = 0.5 * (l2 + m);
 if steep1 { stride = -stride; local = 0.5 * (l1 + m); }
 var cur = uv;
 if horizontal { cur.y += stride * 0.5; } else { cur.x += stride * 0.5; }
 let offset = select(vec2<f32>(0.0, inv.y), vec2<f32>(inv.x, 0.0), horizontal);
 var uv1 = cur - offset; var uv2 = cur + offset;
 var end1 = l(uv1) - local; var end2 = l(uv2) - local;
 var done1 = abs(end1) >= scaled; var done2 = abs(end2) >= scaled;
 if !done1 { uv1 -= offset; }
 if !done2 { uv2 += offset; }
 for (var i = 2u; i < STEPS && !(done1 && done2); i++) {
  if !done1 { end1 = l(uv1) - local; }
  if !done2 { end2 = l(uv2) - local; }
  done1 = abs(end1) >= scaled; done2 = abs(end2) >= scaled;
  if !done1 { uv1 -= offset * quality(i); }
  if !done2 { uv2 += offset * quality(i); }
 }
 let d1 = select(uv.y - uv1.y, uv.x - uv1.x, horizontal);
 let d2 = select(uv2.y - uv.y, uv2.x - uv.x, horizontal);
 let toward1 = d1 < d2;
 let nearest = min(d1, d2);
 let thickness = d1 + d2;
 let pixel_offset = -nearest / max(thickness, 0.000001) + 0.5;
 let smaller = m < local;
 let correct = (select(end2, end1, toward1) < 0.0) != smaller;
 var final_offset = select(0.0, pixel_offset, correct);
 let average = (2.0 * (n + s + e + w) + nw + ne + sw + se) / 12.0;
 let sub1 = clamp(abs(average - m) / range, 0.0, 1.0);
 let sub2 = (-2.0 * sub1 + 3.0) * sub1 * sub1;
 final_offset = max(final_offset, sub2 * sub2 * SUBPIXEL);
 var out_uv = uv;
 if horizontal { out_uv.y += final_offset * stride; } else { out_uv.x += final_offset * stride; }
 return vec4<f32>(at(out_uv), 1.0);
}
