@group(2) @binding(0) var<uniform> material_surface: SurfaceMaterial;

@fragment fn fs_surface(v: Out, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let n = normalize(v.normal) * select(-1.0, 1.0, front);
    let light = g.suns[0].direction_shadow.xyz;
    let color = mesh_decal(textureSample(base, base_sampler, v.uv) * d.color, v.uv, v.world, n)
        * material_surface.tint;
    let result = surface(SurfaceInput(color, v.world, n, v.uv,
        normalize(g.eye_time.xyz - v.world), g.eye_time.w, max(dot(n, light), 0.0),
        mesh_shadow(v.world), material_surface.params));
    let inherit = material_surface.alpha.x < 0.0;
    return surface_alpha(result, select(material_surface.alpha.x, d.params.x, inherit),
        select(material_surface.alpha.y, d.params.y, inherit));
}
