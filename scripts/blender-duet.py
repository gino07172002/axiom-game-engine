"""Generate original, stylized adult dancers and their skinned GLB clips."""
import math
import os
import sys

import bpy
from mathutils import Quaternion, Vector


OUTPUT = os.path.abspath(sys.argv[sys.argv.index("--") + 1])
TAU = math.tau


def dancer(femme):
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for action in list(bpy.data.actions):
        bpy.data.actions.remove(action)

    armature = bpy.data.armatures.new("Duet skeleton")
    rig = bpy.data.objects.new("Dancer", armature)
    bpy.context.collection.objects.link(rig)
    bpy.context.view_layer.objects.active = rig
    rig.select_set(True)
    bpy.ops.object.mode_set(mode="EDIT")

    def bone(name, head, tail, parent=None):
        b = armature.edit_bones.new(name)
        b.head, b.tail = head, tail
        if parent:
            b.parent = armature.edit_bones[parent]

    width = .173 if femme else .211
    bone("root", (0, 0, 0), (0, 0, .2))
    bone("hip", (0, 0, .99), (0, 0, 1.14), "root")
    bone("chest", (0, 0, 1.14), (0, 0, 1.51), "hip")
    bone("head", (0, 0, 1.51), (0, 0, 1.80), "chest")
    limbs = {}
    for side, sign in [("L", 1), ("R", -1)]:
        shoulder = (sign * width, 0, 1.47)
        elbow = (sign * (width + .16), 0, 1.22)
        wrist = (sign * (width + .24), -.035, .98)
        thigh = (sign * .10, 0, 1.00)
        knee = (sign * .115, -.012, .54)
        ankle = (sign * .125, 0, .11)
        limbs[side] = shoulder, elbow, wrist, thigh, knee, ankle
        bone("upper." + side, shoulder, elbow, "chest")
        bone("fore." + side, elbow, wrist, "upper." + side)
        bone("thigh." + side, thigh, knee, "hip")
        bone("shin." + side, knee, ankle, "thigh." + side)
        bone("foot." + side, ankle, (sign * .125, -.12, .06), "shin." + side)
    bpy.ops.object.mode_set(mode="OBJECT")

    palette = {
        "skin": (.72, .36, .26) if femme else (.58, .32, .23),
        "velvet": (.43, .017, .078) if femme else (.046, .032, .085),
        "trim": (.89, .52, .19) if femme else (.21, .15, .33),
        "black": (.021, .015, .043),
        "hair": (.044, .016, .026),
        "shirt": (.86, .75, .77),
        "lip": (.49, .028, .092),
        "eye": (.055, .019, .041),
    }
    materials = []
    for name, color in palette.items():
        mat = bpy.data.materials.new(name)
        mat.use_nodes = True
        bsdf = mat.node_tree.nodes.get("Principled BSDF")
        bsdf.inputs["Base Color"].default_value = (*color, 1)
        bsdf.inputs["Roughness"].default_value = .46 if name == "trim" else .70
        materials.append(mat)

    verts, faces, weights, styles = [], [], [], []

    def shape(rings, u, v, style, group, sides=16):
        start = len(verts)
        for p, rx, ry in rings:
            p = Vector(p)
            for i in range(sides):
                angle = TAU * i / sides
                verts.append(p + u * (math.cos(angle) * rx) + v * (math.sin(angle) * ry))
                weights.append(group)
        index = list(palette).index(style)
        faces.append(tuple(start + i for i in reversed(range(sides))))
        styles.append(index)
        for row in range(len(rings) - 1):
            a, b = start + row * sides, start + (row + 1) * sides
            for i in range(sides):
                j = (i + 1) % sides
                faces.append((a + i, a + j, b + j, b + i))
                styles.append(index)
        end = start + (len(rings) - 1) * sides
        faces.append(tuple(end + i for i in range(sides)))
        styles.append(index)

    def contour(rows, style, group, sides=20):
        shape([((0, y, z), x, d) for z, y, x, d in rows],
              Vector((1, 0, 0)), Vector((0, 1, 0)), style, group, sides)

    def oval(center, radii, style, group, sides=16):
        x, y, z = radii
        rings = []
        for i in range(9):
            a = math.pi * i / 8
            p = Vector(center) + Vector((0, 0, -z * math.cos(a)))
            r = max(.01, math.sin(a))
            rings.append((p, x * r, y * r))
        shape(rings, Vector((1, 0, 0)), Vector((0, 1, 0)), style, group, sides)

    def limb(a, b, radii, style, group, sides=12):
        a, b = Vector(a), Vector(b)
        axis = (b - a).normalized()
        u = axis.cross(Vector((0, 1, 0))).normalized()
        v = axis.cross(u)
        shape([(a.lerp(b, t), r, r * .91) for t, r in zip((0, .23, .73, 1), radii)],
              u, v, style, group, sides)

    contour([(.91, 0, .134, .097), (.99, 0, .172 if femme else .151, .110),
             (1.08, 0, .155, .105), (1.17, 0, .117 if femme else .151, .091)],
            "velvet", "hip")
    if femme:
        contour([(1.11, 0, .120, .093), (1.24, 0, .130, .098),
                 (1.36, -.012, .167, .118), (1.40, -.012, .168, .109)], "velvet", "chest")
        contour([(1.387, 0, .163, .100), (1.45, 0, .173, .086),
                 (1.50, 0, .145, .074), (1.53, 0, .062, .059)], "skin", "chest")
        contour([(.78, .016, .164, .110), (.95, .006, .181, .117),
                 (1.09, 0, .156, .104)], "velvet", "hip")
        contour([(1.075, 0, .159, .108), (1.094, 0, .157, .107)], "trim", "hip")
        limb((.126, -.084, 1.383), (.139, -.028, 1.495),
             (.012, .011, .011, .011), "trim", "chest")
    else:
        contour([(1.11, 0, .149, .093), (1.24, 0, .167, .098),
                 (1.37, -.013, .197, .117), (1.48, 0, width, .083),
                 (1.53, 0, .068, .064)], "velvet", "chest")
        contour([(1.30, -.113, .021, .026), (1.43, -.092, .063, .039),
                 (1.525, -.035, .054, .029)], "shirt", "chest")
        for side in (-1, 1):
            limb((side * .052, -.113, 1.485), (side * .012, -.122, 1.473),
                 (.022, .022, .014, .010), "lip", "chest")
    oval((0, 0, 1.53), (.061, .059, .080), "skin", "head")
    if femme:
        contour([(1.548, 0, .063, .063), (1.565, 0, .062, .062)], "black", "head")
    oval((0, -.003, 1.662), (.100 if femme else .106, .089, .130), "skin", "head", 24)
    oval((0, .029, 1.730), (.104 if femme else .110, .091, .078), "hair", "head", 24)
    if femme:
        oval((.091, .064, 1.666), (.061, .082, .128), "hair", "head")
        oval((-.084, -.006, 1.603), (.014, .015, .027), "trim", "head", 10)
    for sign in (-1, 1):
        oval((sign * .040, -.083, 1.672), (.024, .010, .007), "eye", "head", 10)
    oval((0, -.096, 1.641), (.015, .020, .020), "skin", "head", 10)
    oval((0, -.079, 1.608), (.025, .008, .006), "lip", "head", 12)

    for side, (shoulder, elbow, wrist, thigh, knee, ankle) in limbs.items():
        sign = 1 if side == "L" else -1
        limb(shoulder, elbow, (.070, .066, .052, .048), "skin" if femme else "velvet", "upper." + side)
        limb(elbow, wrist, (.049, .047, .038, .031), "black" if femme else "velvet", "fore." + side)
        oval((wrist[0] + .015 * sign, wrist[1] - .007, wrist[2] - .044),
             (.038, .026, .071), "black" if femme else "skin", "fore." + side)
        limb(thigh, knee, (.088, .086, .070, .058), "skin" if femme else "velvet", "thigh." + side)
        limb(knee, ankle, (.059, .064, .053, .043), "black", "shin." + side)
        oval((ankle[0], -.060, .077), (.057, .109, .062), "black", "foot." + side)
        if femme:
            oval((ankle[0], .022, .045), (.028, .027, .042), "trim", "foot." + side, 10)

    mesh = bpy.data.meshes.new("Tailored evening wear")
    mesh.from_pydata(verts, [], faces)
    obj = bpy.data.objects.new("Skin", mesh)
    bpy.context.collection.objects.link(obj)
    obj.parent = rig
    for mat in materials:
        mesh.materials.append(mat)
    for poly, index in zip(mesh.polygons, styles):
        poly.material_index = index
        poly.use_smooth = True
    for name in sorted(set(weights)):
        group = obj.vertex_groups.new(name=name)
        group.add([i for i, n in enumerate(weights) if n == name], 1.0, "REPLACE")
    modifier = obj.modifiers.new("Skeleton", "ARMATURE")
    modifier.object = rig

    scene = bpy.context.scene
    scene.render.fps = 30
    scene.frame_start, scene.frame_end = 1, 25
    rig.animation_data_create()
    mirror = 1 if femme else -1
    clips = {
        "idle": (.14, .23, .14, .04, .14),
        "sway": (0, .65, .45, -.30, 0),
        "turn": (0, .50, .85, .35, 0),
        "reach": (0, .55, .90, .70, 0),
        "finale": (0, .50, .88, 1.0, 1.0),
    }
    for name, weights in clips.items():
        action = bpy.data.actions.new(name)
        rig.animation_data.action = action
        for i, w in enumerate(weights):
            frame = 1 + 6 * i
            for pose in rig.pose.bones:
                pose.rotation_mode = "QUATERNION"
                pose.rotation_quaternion = Quaternion()
            angles = {
                "hip": (0, mirror * 15 * w, mirror * 17 * w),
                "chest": (10 * w, -mirror * 12 * w, -mirror * 20 * w),
                "head": (-7 * w, mirror * 13 * w, mirror * 16 * w),
                "upper.L": (-24 * w, -53 * w, -18 * w),
                "upper.R": (-28 * w, 62 * w, 18 * w),
                "fore.L": (-36 * w, 12 * w, 0),
                "fore.R": (-32 * w, -7 * w, 0),
                "thigh.L": (-15 * w, -4 * w, 0),
                "shin.L": (23 * w, 0, 0),
                "thigh.R": (8 * w, 6 * w, 0),
            }
            if name == "turn":
                angles["root"] = (0, 0, 90 * i * mirror)
            if name == "reach":
                angles["upper.L" if femme else "upper.R"] = (-66 * w, -35 * w * mirror, 0)
            if name == "finale":
                angles["upper.L"] = (-25 * w, -115 * w, -30 * w)
                angles["upper.R"] = (-28 * w, 110 * w, 28 * w)
            for pose in rig.pose.bones:
                q = Quaternion()
                basis = pose.bone.matrix_local.to_quaternion().inverted()
                for axis, angle in zip(((1, 0, 0), (0, 1, 0), (0, 0, 1)), angles.get(pose.name, (0, 0, 0))):
                    q = q @ Quaternion(basis @ Vector(axis), math.radians(angle))
                pose.rotation_quaternion = q
                pose.keyframe_insert("rotation_quaternion", frame=frame, group=pose.name)
        track = rig.animation_data.nla_tracks.new()
        track.name = name
        track.strips.new(name, 1, action)
        track.mute = True
    rig.animation_data.action = None
    scene.frame_set(1)
    bpy.ops.object.select_all(action="DESELECT")
    rig.select_set(True)
    obj.select_set(True)
    bpy.context.view_layer.objects.active = rig
    path = os.path.join(OUTPUT, "valentina.glb" if femme else "ren.glb")
    bpy.ops.export_scene.gltf(filepath=path, export_format="GLB", use_selection=True,
        export_animations=True, export_frame_range=True, export_force_sampling=True,
        export_animation_mode="ACTIONS", export_anim_single_armature=True, export_skins=True)


if __name__ == "__main__":
    os.makedirs(OUTPUT, exist_ok=True)
    dancer(True)
    dancer(False)
