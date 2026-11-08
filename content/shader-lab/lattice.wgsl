fn surface(s: SurfaceInput) -> vec4<f32> {
    let cell = abs(fract(s.uv * 5.0) - vec2<f32>(0.5));
    let strut = step(0.34, max(cell.x, cell.y));
    return vec4<f32>(s.base_color.rgb * (0.68 + 0.32 * s.diffuse), strut);
}
