"""Create an original armored adult fighter and sampled melee clips."""
import importlib.util
import math
import os
import sys

import bpy
from mathutils import Quaternion, Vector

out = os.path.abspath(sys.argv[sys.argv.index("--") + 1])
os.makedirs(out, exist_ok=True)
spec = importlib.util.spec_from_file_location("duet_shapes", os.path.join(os.path.dirname(__file__), "blender-duet.py"))
shapes = importlib.util.module_from_spec(spec)
spec.loader.exec_module(shapes)
# Reuse our own mesh generator as a base, then replace the outfit and all actions.
shapes.OUTPUT = out
shapes.dancer(False)
rig = bpy.data.objects["Dancer"]
skin = bpy.data.objects["Skin"]
palette = {
    "velvet": (.040, .12, .16),
    "shirt": (.14, .30, .32),
    "black": (.022, .042, .052),
    "trim": (.84, .38, .12),
    "lip": (.12, .035, .022),
    "hair": (.018, .027, .032),
}
for material in skin.data.materials:
    if material.name in palette:
        material.node_tree.nodes["Principled BSDF"].inputs["Base Color"].default_value = (*palette[material.name], 1)

def armor(name, position, scale, bone, color, angle=0):
    bpy.ops.mesh.primitive_cube_add(size=1, location=position)
    obj = bpy.context.object
    obj.name = name
    obj.scale = scale
    obj.rotation_euler.z = angle
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    mat.node_tree.nodes["Principled BSDF"].inputs["Base Color"].default_value = (*color, 1)
    obj.data.materials.append(mat)
    obj.parent = rig
    group = obj.vertex_groups.new(name=bone)
    group.add(list(range(len(obj.data.vertices))), 1, "REPLACE")
    modifier = obj.modifiers.new("Armor skin", "ARMATURE")
    modifier.object = rig

armor("Breastplate", (0, -.075, 1.335), (.30, .13, .26), "chest", (.10, .33, .37))
armor("Belt", (0, -.005, 1.08), (.32, .23, .045), "hip", (.82, .38, .13))
for side, sign in [("L", 1), ("R", -1)]:
    armor("Shoulder " + side, (sign * .225, 0, 1.465), (.18, .19, .12), "upper." + side, (.23, .42, .43))
    armor("Bracer " + side, (sign * .422, -.02, 1.04), (.09, .13, .19), "fore." + side, (.14, .28, .31))
armor("Sword grip", (.456, -.03, .93), (.032, .045, .16), "fore.L", (.07, .05, .035))
armor("Crossguard", (.456, -.03, .99), (.18, .055, .032), "fore.L", (.95, .48, .16))
armor("Blade", (.456, -.03, 1.29), (.050, .025, .59), "fore.L", (.54, .81, .86))

for track in list(rig.animation_data.nla_tracks):
    rig.animation_data.nla_tracks.remove(track)
rig.animation_data.action = None
for action in list(bpy.data.actions):
    bpy.data.actions.remove(action)
clips = ["idle", "run", "slash1", "slash2", "slash3", "dodge", "enemy", "death"]
for name in clips:
    action = bpy.data.actions.new(name)
    rig.animation_data.action = action
    for i in range(9):
        u = i / 8
        pulse = math.sin(u * math.pi)
        swing = math.sin((u - .3) * math.pi * 2) * pulse
        angles = {
            "chest": (0, 0, 0),
            "upper.L": (-13, -24, -16),
            "fore.L": (-37, -8, 0),
            "upper.R": (-10, 23, 10),
            "fore.R": (-18, 0, 0),
        }
        if name == "idle":
            angles["chest"] = (2 * math.sin(u * math.tau), 0, 0)
        if name == "run":
            stride = math.sin(u * math.tau)
            angles.update({"thigh.L": (34 * stride, 0, 0), "thigh.R": (-34 * stride, 0, 0),
                           "shin.L": (max(0, -stride) * 50, 0, 0), "shin.R": (max(0, stride) * 50, 0, 0),
                           "upper.R": (25 * stride, 10, 5), "chest": (8, 0, 0)})
        if name in ("slash1", "slash2", "slash3", "enemy"):
            sign = -1 if name == "slash2" else 1
            angles.update({"chest": (9 * pulse, 0, sign * 58 * swing),
                           "hip": (0, 0, -sign * 18 * swing),
                           "upper.L": (-112 * pulse - 13, -32 - 48 * pulse, sign * 68 * swing),
                           "fore.L": (-65 * (1 - u) * pulse, -15 * pulse, -50 * swing),
                           "upper.R": (-38 * pulse - 12, 34, -sign * 26 * swing),
                           "thigh.R": (-18 * pulse, 0, 0), "shin.R": (28 * pulse, 0, 0)})
            if name == "slash3":
                angles["upper.L"] = (-148 * pulse, -44, 16)
                angles["chest"] = (28 * swing, 0, 0)
        if name == "dodge":
            angles.update({"root": (-360 * u, 0, 0), "hip": (28 * pulse, 0, 0),
                           "thigh.L": (-78 * pulse, 0, 0), "thigh.R": (-62 * pulse, 0, 0),
                           "shin.L": (100 * pulse, 0, 0), "shin.R": (105 * pulse, 0, 0),
                           "upper.L": (-38, -15, 0), "upper.R": (-38, 15, 0)})
        if name == "death":
            angles.update({"root": (-80 * min(1, u * 2), 0, 0), "upper.L": (0, -15, 0),
                           "upper.R": (0, 15, 0), "head": (20 * pulse, 0, 0)})
        for pose in rig.pose.bones:
            pose.rotation_mode = "QUATERNION"
            basis = pose.bone.matrix_local.to_quaternion().inverted()
            q = Quaternion()
            for axis, angle in zip(((1, 0, 0), (0, 1, 0), (0, 0, 1)), angles.get(pose.name, (0, 0, 0))):
                q = q @ Quaternion(basis @ Vector(axis), math.radians(angle))
            pose.rotation_quaternion = q
            pose.location = Vector((0, 0, 0))
            if pose.name == "root":
                if name == "dodge":
                    pose.location = Vector((0, .7 * pulse, 0))
                if name == "death":
                    pose.location = Vector((0, .04, 0))
            pose.keyframe_insert("rotation_quaternion", frame=1 + i * 3)
            pose.keyframe_insert("location", frame=1 + i * 3)
    track = rig.animation_data.nla_tracks.new()
    track.name = name
    track.strips.new(name, 1, action)
    track.mute = True
rig.animation_data.action = None
bpy.context.scene.frame_set(1)
bpy.ops.object.select_all(action="DESELECT")
rig.select_set(True)
for obj in bpy.data.objects:
    if obj.parent == rig:
        obj.select_set(True)
bpy.context.view_layer.objects.active = rig
bpy.ops.export_scene.gltf(filepath=os.path.join(out, "fighter.glb"), export_format="GLB", use_selection=True,
    export_animations=True, export_frame_range=True, export_force_sampling=True,
    export_animation_mode="ACTIONS", export_anim_single_armature=True, export_skins=True)
# A second prepared asset exercises model replacement and distinguishes opponents.
for material in bpy.data.materials:
    if not material.use_nodes:
        continue
    bsdf = material.node_tree.nodes.get("Principled BSDF")
    if material.name in ("velvet", "Breastplate", "Bracer L", "Bracer R"):
        bsdf.inputs["Base Color"].default_value = (.43, .085, .045, 1)
    if material.name.startswith("Shoulder"):
        bsdf.inputs["Base Color"].default_value = (.55, .23, .09, 1)
for obj in bpy.context.selected_objects:
    if obj.type == "MESH":
        for vertex in obj.data.vertices:
            vertex.co.x *= 1.05
bpy.ops.export_scene.gltf(filepath=os.path.join(out, "fighter-alt.glb"), export_format="GLB", use_selection=True,
    export_animations=True, export_frame_range=True, export_force_sampling=True,
    export_animation_mode="ACTIONS", export_anim_single_armature=True, export_skins=True)
# The shared base generator emitted this intermediate file.
os.remove(os.path.join(out, "ren.glb"))
