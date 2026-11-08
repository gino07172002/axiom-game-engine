"""Create our own tiny skinned Rigify fixture with Blender; never reads/copies reference artwork."""
import bpy, json, math, os, sys
from mathutils import Vector, Quaternion

out = os.path.abspath(sys.argv[sys.argv.index('--') + 1])
os.makedirs(out, exist_ok=True)
bpy.ops.object.select_all(action='SELECT')
bpy.ops.object.delete(use_global=False)
bpy.ops.preferences.addon_enable(module='rigify')
bpy.ops.object.armature_human_metarig_add()
meta = bpy.context.object
bpy.ops.pose.rigify_generate()
rig = bpy.context.object
assert rig.type == 'ARMATURE' and rig != meta
for bone in rig.pose.bones:
    if 'IK_FK' in bone:
        bone['IK_FK'] = 1.0
bpy.context.scene.render.fps = 30
bpy.context.scene.frame_start = 1
bpy.context.scene.frame_end = 31
control = rig.pose.bones['upper_arm_fk.L']
control.rotation_mode = 'QUATERNION'
for frame, angle in [(1, 0), (16, 0.7), (31, 0)]:
    control.rotation_quaternion = Quaternion((0, 0, 1), angle)
    control.keyframe_insert('rotation_quaternion', frame=frame)
rig.animation_data.action.name = 'rigify_wave'
# Skin rectangular bars to the actual generated deform bones.
verts, faces, groups = [], [], []
for bone in rig.data.bones:
    if not bone.use_deform or bone.length < .035:
        continue
    start = len(verts)
    axis = bone.tail_local - bone.head_local
    axis.normalize()
    u = axis.cross(Vector((0, 0, 1)))
    if u.length < .01:
        u = axis.cross(Vector((1, 0, 0)))
    u.normalize()
    v = axis.cross(u).normalized()
    radius = .018
    for p in [bone.head_local, bone.tail_local]:
        for x, y in [(-1, -1), (1, -1), (1, 1), (-1, 1)]:
            verts.append(tuple(p + radius * (u*x + v*y)))
            groups.append(bone.name)
    for f in [(0,3,2,1),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)]:
        faces.append(tuple(start+i for i in f))
mesh = bpy.data.meshes.new('fixture_mesh')
mesh.from_pydata(verts, [], faces)
obj = bpy.data.objects.new('fixture_skin', mesh)
bpy.context.collection.objects.link(obj)
for name in set(groups):
    group = obj.vertex_groups.new(name=name)
    group.add([i for i,n in enumerate(groups) if n == name], 1, 'REPLACE')
modifier = obj.modifiers.new('armature', 'ARMATURE')
modifier.object = rig
obj.parent = rig
# Capture evaluated bone origins independently from the Rust glTF importer.
expected = []
for frame in [1,16,31]:
    bpy.context.scene.frame_set(frame)
    bpy.context.view_layer.update()
    pose = {}
    for name in ['DEF-spine','DEF-upper_arm.L','DEF-forearm.L','DEF-hand.L','DEF-thigh.L','DEF-shin.L','DEF-foot.L']:
        p = rig.matrix_world @ rig.pose.bones[name].matrix.translation
        pose[name] = [p.x,p.z,-p.y]  # Blender glTF exporter axis convention.
    expected.append({'seconds':(frame-1)/30, 'positions':pose})
assert (Vector(expected[1]['positions']['DEF-hand.L']) - Vector(expected[0]['positions']['DEF-hand.L'])).length > .05, 'Rigify fixture must contain real deform motion'
bpy.context.scene.frame_set(1)
bpy.ops.object.select_all(action='DESELECT')
rig.select_set(True)
obj.select_set(True)
bpy.context.view_layer.objects.active = rig
bpy.ops.export_scene.gltf(filepath=os.path.join(out,'rigify.glb'),export_format='GLB',use_selection=True,
    export_animations=True,export_frame_range=True,export_force_sampling=True,export_def_bones=True,
    export_animation_mode='ACTIONS',export_anim_single_armature=True,export_skins=True)
with open(os.path.join(out,'rigify-expected.json'),'w',encoding='utf8') as f:
    json.dump({'blender':bpy.app.version_string,'expected':expected,'deform_bones':[b.name for b in rig.data.bones if b.use_deform]},f,indent=2)
print('AXIOM_RIGIFY_FIXTURE_OK',out)
