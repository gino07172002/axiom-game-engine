struct SurfaceMaterial {
    tint: vec4<f32>,
    params: vec4<f32>,
    alpha: vec4<f32>,
}

fn surface_alpha(color: vec4<f32>, mode: f32, cutoff: f32) -> vec4<f32> {
    if mode == 1.0 && color.a < cutoff { discard; }
    return vec4<f32>(color.rgb, select(1.0, clamp(color.a, 0.0, 1.0), mode == 2.0));
}
