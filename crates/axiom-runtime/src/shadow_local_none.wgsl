// Shaders without the local shadow maps (VFX particles): every local light is unshadowed.
fn axiom_local_shadow(l: LocalLight, p: vec3<f32>, n: vec3<f32>) -> f32 { return 1.0; }
