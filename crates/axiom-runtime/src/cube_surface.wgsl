@group(1) @binding(0) var<uniform> material_surface: SurfaceMaterial;

@fragment fn fs_surface(v: Out, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let n = normalize(v.normal) * select(-1.0, 1.0, front);
    let light = normalize(vec3<f32>(-0.45, 0.85, 0.35));
    let a = abs(n);
    var uv = v.world.xy;
    if a.y >= max(a.x, a.z) { uv = v.world.xz; }
    else if a.x >= a.z { uv = v.world.zy; }
    let result = surface(SurfaceInput(v.color * material_surface.tint, v.world, n, uv,
        normalize(g.eye_time.xyz - v.world), g.eye_time.w, max(dot(n, light), 0.0),
        scene_shadow(v.light), material_surface.params));
    return surface_alpha(result, max(0.0, material_surface.alpha.x), material_surface.alpha.y);
}
