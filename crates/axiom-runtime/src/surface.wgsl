struct SurfaceInput {
    base_color: vec4<f32>,
    world_position: vec3<f32>,
    normal: vec3<f32>,
    uv: vec2<f32>,
    view_direction: vec3<f32>,
    time: f32,
    diffuse: f32,
    shadow: f32,
    params: vec4<f32>,
}

fn axiom_local_light(p:vec3<f32>,n:vec3<f32>)->vec3<f32> {return vec3<f32>(0.0);}
