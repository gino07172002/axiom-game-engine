"""Builds content/rig-lab/puppet.glb: a first-party procedural test character for spring bones,
IK sockets and look-at (python3 standard library only, deterministic output).

Skeleton (rest rotations are identity): root > body > neck > head > hair.0..hair.5 (a ponytail
hanging behind the head), body > upper/fore/hand .L/.R (two-bone IK arms), root > legs. Every box
is rigidly skinned to one bone. One clip, `sway`: the body turns and the root slides so the
ponytail swings. Run: python scripts/build-rig-lab.py
"""
import json
import math
import os
import struct

BONES = [
    # name, parent, global rest position
    ("root", None, (0.0, 0.0, 0.0)),
    ("body", "root", (0.0, 0.55, 0.0)),
    ("neck", "body", (0.0, 1.05, 0.0)),
    ("head", "neck", (0.0, 1.13, 0.0)),
    ("hair.0", "head", (0.0, 1.36, -0.12)),
    ("hair.1", "hair.0", (0.0, 1.28, -0.15)),
    ("hair.2", "hair.1", (0.0, 1.20, -0.17)),
    ("hair.3", "hair.2", (0.0, 1.12, -0.18)),
    ("hair.4", "hair.3", (0.0, 1.04, -0.185)),
    ("hair.5", "hair.4", (0.0, 0.96, -0.19)),
    ("upper.L", "body", (0.21, 1.0, 0.0)),
    ("fore.L", "upper.L", (0.21, 0.76, 0.0)),
    ("hand.L", "fore.L", (0.21, 0.53, 0.0)),
    ("upper.R", "body", (-0.21, 1.0, 0.0)),
    ("fore.R", "upper.R", (-0.21, 0.76, 0.0)),
    ("hand.R", "fore.R", (-0.21, 0.53, 0.0)),
]
INDEX = {b[0]: i for i, b in enumerate(BONES)}
MATERIALS = [
    ("cloth", (0.18, 0.5, 0.78, 1.0)),
    ("skin", (0.96, 0.78, 0.66, 1.0)),
    ("hair", (0.92, 0.36, 0.14, 1.0)),
    ("dark", (0.14, 0.14, 0.18, 1.0)),
]


def box(lo, hi, bone, material, out):
    """Axis-aligned box with flat normals, rigidly skinned to `bone`."""
    (x0, y0, z0), (x1, y1, z1) = lo, hi
    faces = [
        ((1, 0, 0), [(x1, y0, z0), (x1, y1, z0), (x1, y1, z1), (x1, y0, z1)]),
        ((-1, 0, 0), [(x0, y0, z1), (x0, y1, z1), (x0, y1, z0), (x0, y0, z0)]),
        ((0, 1, 0), [(x0, y1, z0), (x0, y1, z1), (x1, y1, z1), (x1, y1, z0)]),
        ((0, -1, 0), [(x0, y0, z1), (x0, y0, z0), (x1, y0, z0), (x1, y0, z1)]),
        ((0, 0, 1), [(x0, y0, z1), (x1, y0, z1), (x1, y1, z1), (x0, y1, z1)]),
        ((0, 0, -1), [(x1, y0, z0), (x0, y0, z0), (x0, y1, z0), (x1, y1, z0)]),
    ]
    prim = out[material]
    for normal, quad in faces:
        base = len(prim["pos"])
        for p in quad:
            prim["pos"].append(p)
            prim["nrm"].append(normal)
            prim["jnt"].append((INDEX[bone], 0, 0, 0))
            prim["wgt"].append((1.0, 0.0, 0.0, 0.0))
        prim["idx"] += [base, base + 1, base + 2, base, base + 2, base + 3]


def segment(a, b, width, bone, material, out):
    """A box from bone head `a` to `b` (both on a near-vertical line), `width` thick."""
    lo = tuple(min(a[i], b[i]) - width / 2 for i in range(3))
    hi = tuple(max(a[i], b[i]) + width / 2 for i in range(3))
    box(lo, hi, bone, material, out)


def build_mesh():
    out = {m: {"pos": [], "nrm": [], "jnt": [], "wgt": [], "idx": []} for m, _ in MATERIALS}
    pos = {b[0]: b[2] for b in BONES}
    box((-0.17, 0.5, -0.1), (0.17, 1.05, 0.1), "body", "cloth", out)
    box((-0.15, 0.0, -0.08), (-0.03, 0.5, 0.08), "root", "dark", out)
    box((0.03, 0.0, -0.08), (0.15, 0.5, 0.08), "root", "dark", out)
    box((-0.04, 1.03, -0.04), (0.04, 1.14, 0.04), "neck", "skin", out)
    box((-0.13, 1.13, -0.12), (0.13, 1.39, 0.12), "head", "skin", out)
    box((-0.135, 1.33, -0.125), (0.135, 1.41, 0.125), "head", "hair", out)
    box((-0.025, 1.22, 0.12), (0.025, 1.27, 0.17), "head", "dark", out)  # nose: facing +Z
    box((-0.08, 1.28, 0.115), (-0.04, 1.31, 0.13), "head", "dark", out)
    box((0.04, 1.28, 0.115), (0.08, 1.31, 0.13), "head", "dark", out)
    hair = [f"hair.{i}" for i in range(6)]
    for i in range(5):
        segment(pos[hair[i]], pos[hair[i + 1]], 0.07 - i * 0.008, hair[i], "hair", out)
    for side in "LR":
        segment(pos[f"upper.{side}"], pos[f"fore.{side}"], 0.08, f"upper.{side}", "cloth", out)
        segment(pos[f"fore.{side}"], pos[f"hand.{side}"], 0.07, f"fore.{side}", "skin", out)
        h = pos[f"hand.{side}"]
        box((h[0] - 0.04, h[1] - 0.1, h[2] - 0.04), (h[0] + 0.04, h[1], h[2] + 0.04), f"hand.{side}", "skin", out)
    return out


def quat_y(angle):
    return (0.0, math.sin(angle / 2), 0.0, math.cos(angle / 2))


def quat_mul(a, b):
    ax, ay, az, aw = a
    bx, by, bz, bw = b
    return (
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    )


def quat_z(angle):
    return (0.0, 0.0, math.sin(angle / 2), math.cos(angle / 2))


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    target = os.path.join(here, "..", "content", "rig-lab", "puppet.glb")
    mesh = build_mesh()
    blob = bytearray()
    views, accessors = [], []

    def add(data, target_kind=None):
        while len(blob) % 4:
            blob.append(0)
        view = {"buffer": 0, "byteOffset": len(blob), "byteLength": len(data)}
        if target_kind:
            view["target"] = target_kind
        blob.extend(data)
        views.append(view)
        return len(views) - 1

    def accessor(values, fmt, ctype, kind, minmax=False, target_kind=None):
        data = b"".join(struct.pack("<" + fmt, *v) if isinstance(v, tuple) else struct.pack("<" + fmt, v) for v in values)
        a = {"bufferView": add(data, target_kind), "componentType": ctype, "count": len(values), "type": kind}
        if minmax:
            n = len(values[0])
            a["min"] = [min(v[i] for v in values) for i in range(n)]
            a["max"] = [max(v[i] for v in values) for i in range(n)]
        accessors.append(a)
        return len(accessors) - 1

    primitives, materials = [], []
    for i, (name, color) in enumerate(MATERIALS):
        p = mesh[name]
        materials.append({"name": name, "pbrMetallicRoughness": {"baseColorFactor": list(color), "metallicFactor": 0.0, "roughnessFactor": 0.8}})
        primitives.append({
            "attributes": {
                "POSITION": accessor(p["pos"], "fff", 5126, "VEC3", True, 34962),
                "NORMAL": accessor(p["nrm"], "fff", 5126, "VEC3", False, 34962),
                "JOINTS_0": accessor(p["jnt"], "HHHH", 5123, "VEC4", False, 34962),
                "WEIGHTS_0": accessor(p["wgt"], "ffff", 5126, "VEC4", False, 34962),
            },
            "indices": accessor(p["idx"], "H", 5123, "SCALAR", False, 34963),
            "material": i,
        })
    nodes = []
    for name, parent, p in BONES:
        local = p if parent is None else tuple(p[k] - BONES[INDEX[parent]][2][k] for k in range(3))
        nodes.append({"name": name, "translation": [round(v, 6) for v in local]})
    for i, (name, parent, _) in enumerate(BONES):
        if parent is not None:
            nodes[INDEX[parent]].setdefault("children", []).append(i)
    inverse = []
    for _, _, p in BONES:
        inverse.append((1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, -p[0], -p[1], -p[2], 1))
    ibm = accessor(inverse, "f" * 16, 5126, "MAT4")
    nodes.append({"name": "puppet-mesh", "mesh": 0, "skin": 0})
    # sway: 2 s loop, body turns +-35 degrees and tilts, the root slides +-0.12 m.
    keys = 24
    times = [2.0 * k / keys for k in range(keys + 1)]
    turn = [quat_mul(quat_y(math.radians(35) * math.sin(2 * math.pi * t / 2.0)), quat_z(math.radians(6) * math.sin(4 * math.pi * t / 2.0))) for t in times]
    slide = [(0.12 * math.sin(2 * math.pi * t / 2.0 + 0.6), 0.0, 0.0) for t in times]
    t_acc = accessor(times, "f", 5126, "SCALAR")
    accessors[t_acc]["min"] = [times[0]]
    accessors[t_acc]["max"] = [times[-1]]
    r_acc = accessor(turn, "ffff", 5126, "VEC4")
    s_acc = accessor(slide, "fff", 5126, "VEC3")
    animation = {
        "name": "sway",
        "samplers": [
            {"input": t_acc, "output": r_acc, "interpolation": "LINEAR"},
            {"input": t_acc, "output": s_acc, "interpolation": "LINEAR"},
        ],
        "channels": [
            {"sampler": 0, "target": {"node": INDEX["body"], "path": "rotation"}},
            {"sampler": 1, "target": {"node": INDEX["root"], "path": "translation"}},
        ],
    }
    doc = {
        "asset": {"version": "2.0", "generator": "axiom scripts/build-rig-lab.py"},
        "scene": 0,
        "scenes": [{"nodes": [INDEX["root"], len(nodes) - 1]}],
        "nodes": nodes,
        "meshes": [{"name": "puppet", "primitives": primitives}],
        "materials": materials,
        "skins": [{"joints": list(range(len(BONES))), "inverseBindMatrices": ibm, "skeleton": INDEX["root"]}],
        "animations": [animation],
        "accessors": accessors,
        "bufferViews": views,
        "buffers": [{"byteLength": len(blob)}],
    }
    text = json.dumps(doc, separators=(",", ":"), sort_keys=True).encode()
    while len(text) % 4:
        text += b" "
    while len(blob) % 4:
        blob.append(0)
    total = 12 + 8 + len(text) + 8 + len(blob)
    os.makedirs(os.path.dirname(target), exist_ok=True)
    with open(target, "wb") as f:
        f.write(struct.pack("<III", 0x46546C67, 2, total))
        f.write(struct.pack("<II", len(text), 0x4E4F534A) + text)
        f.write(struct.pack("<II", len(blob), 0x004E4942) + bytes(blob))
    print(os.path.normpath(target), total, "bytes")


if __name__ == "__main__":
    main()
