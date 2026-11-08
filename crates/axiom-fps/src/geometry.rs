use super::*;
pub fn arena_parts(world: &LegacyWorld) -> Vec<MeshPart> {
    let mut out = Vec::with_capacity(1200);
    let h = world.arena.half_size;
    let teal = world.arena.accent;
    let stone = [0.25, 0.31, 0.34];
    let dark = [0.055, 0.085, 0.10];
    let orange = [1.0, 0.38, 0.12];
    let mut add = |p: Vec3, s: Vec3, q: Quat, color: [f32; 3], emission: f32, grid: f32| {
        out.push(MeshPart {
            transform: Transform {
                position: p.to_array(),
                rotation: q.to_array(),
                scale: s.to_array(),
            },
            color,
            emission,
            grid: grid > 0.5,
        });
    };
    let id = Quat::IDENTITY;
    add(
        Vec3::new(0.0, -0.25, 0.0),
        Vec3::new(h * 2.0, 0.5, h * 2.0),
        id,
        [0.11, 0.15, 0.17],
        0.0,
        1.0,
    );
    for side in 0..4 {
        let q = Quat::from_rotation_y(side as f32 * std::f32::consts::FRAC_PI_2);
        let height = world.arena.wall_height;
        add(
            q * Vec3::new(0.0, height * 0.5, -h - 0.45),
            Vec3::new(h * 2.0, height, 0.9),
            q,
            [0.14, 0.20, 0.24],
            0.0,
            0.0,
        );
        add(
            q * Vec3::new(0.0, 0.15, -h + 0.02),
            Vec3::new(h * 2.0, 0.08, 0.09),
            q,
            teal,
            2.2,
            0.0,
        );
        add(
            q * Vec3::new(0.0, height - 0.25, -h + 0.02),
            Vec3::new(h * 2.0, 0.06, 0.08),
            q,
            teal,
            1.4,
            0.0,
        );
        for n in -5..=5 {
            let x = n as f32 * h / 5.5;
            add(
                q * Vec3::new(x, height * 0.5, -h + 0.15),
                Vec3::new(0.22, height, 0.55),
                q,
                stone,
                0.0,
                0.0,
            );
            add(
                q * Vec3::new(x, height + 0.8, -h - 0.2),
                Vec3::new(0.5, 1.6, 0.8),
                q,
                dark,
                0.0,
                0.0,
            );
            if n % 2 == 0 {
                add(
                    q * Vec3::new(x, 2.4, -h + 0.33),
                    Vec3::new(0.08, 1.0, 0.08),
                    q,
                    teal,
                    2.5,
                    0.0,
                );
            }
        }
    }

    // Inlaid lane markers and perimeter hazard strips.
    for z in -8..=8 {
        for x in [-2.0, 2.0] {
            add(
                Vec3::new(x, 0.006, z as f32 * 2.0),
                Vec3::new(0.055, 0.012, 0.65),
                id,
                teal,
                0.5,
                0.0,
            );
        }
    }
    for x in -12..=12 {
        add(
            Vec3::new(x as f32 * 0.52, 0.008, 16.0),
            Vec3::new(0.25, 0.014, 0.7),
            Quat::from_rotation_y(0.5),
            orange,
            0.15,
            0.0,
        );
    }
    // Arena markings are content geometry, rather than a global renderer effect.
    let radius = 9.0 * h / 22.0;
    for i in 0..128 {
        let angle = i as f32 * std::f32::consts::TAU / 128.0;
        out.push(MeshPart {
            transform: Transform {
                position: [angle.sin() * radius, 0.009, angle.cos() * radius],
                rotation: Quat::from_rotation_y(angle + std::f32::consts::FRAC_PI_2).to_array(),
                scale: [0.08, 0.012, std::f32::consts::TAU * radius / 128.0 + 0.006],
            },
            color: teal,
            emission: 0.5,
            grid: false,
        });
    }
    out
}
pub fn cover_parts(o: &Obstacle, teal: [f32; 3]) -> Vec<MeshPart> {
    let mut out = Vec::new();
    let stone = [0.25, 0.31, 0.34];
    let dark = [0.055, 0.085, 0.10];
    let id = Quat::IDENTITY;
    let mut add = |p: Vec3, s: Vec3, q: Quat, color: [f32; 3], emission: f32, grid: f32| {
        out.push(MeshPart {
            transform: Transform {
                position: p.to_array(),
                rotation: q.to_array(),
                scale: s.to_array(),
            },
            color,
            emission,
            grid: grid > 0.5,
        });
    };

    let p = Vec3::ZERO;
    let s = Vec3::from_array(o.size);
    add(p, s, id, stone, 0.0, 0.0);
    add(
        p + Vec3::Y * (s.y * 0.5 + 0.04),
        Vec3::new(s.x + 0.12, 0.08, s.z + 0.12),
        id,
        [0.39, 0.45, 0.45],
        0.0,
        0.0,
    );
    for sign in [-1.0, 1.0] {
        add(
            p + Vec3::new(0.0, s.y * 0.2, sign * (s.z * 0.5 + 0.015)),
            Vec3::new(s.x * 0.82, 0.075, 0.035),
            id,
            teal,
            1.5,
            0.0,
        );
        add(
            p + Vec3::new(sign * (s.x * 0.5 + 0.015), s.y * 0.2, 0.0),
            Vec3::new(0.035, 0.075, s.z * 0.82),
            id,
            teal,
            1.5,
            0.0,
        );
    }
    if o.id == "reactor" {
        add(
            p + Vec3::Y * (s.y * 0.5 + 0.65),
            Vec3::new(0.8, 1.15, 0.8),
            Quat::from_rotation_y(0.0),
            teal,
            2.5,
            0.0,
        );
        for sign in [-1.0, 1.0] {
            add(
                p + Vec3::new(sign * 0.7, s.y * 0.5 + 0.25, 0.0),
                Vec3::new(0.15, 0.5, 0.15),
                id,
                dark,
                0.0,
                0.0,
            );
        }
    }
    out
}
