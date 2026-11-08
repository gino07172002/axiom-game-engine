fn surface(s: SurfaceInput) -> vec4<f32> {
    let scan = fract(s.world_position.y * s.params.x - s.time * s.params.y);
    let stripe = smoothstep(0.65, 0.9, scan);
    let rim = pow(1.0 - clamp(dot(s.view_direction, s.normal), 0.0, 1.0), 3.0);
    let color = s.base_color.rgb * (0.35 + 0.65 * stripe + 0.2 * s.diffuse);
    return vec4<f32>(color + rim * vec3<f32>(0.0, 0.2, 0.32), s.base_color.a);
}
