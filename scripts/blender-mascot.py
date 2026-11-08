"""Generate the original Axiom mascot (short bob / long hair) as skinned toon GLBs.

Usage: blender --background --python scripts/blender-mascot.py -- <outdir>

The skeleton, bone names and clip names follow scripts/blender-duet.py so the
runtime animation contract is unchanged: root, hip, chest, head, upper.L/R,
fore.L/R, thigh.L/R, shin.L/R, foot.L/R and the clips idle, sway, turn, reach,
finale. Everything is procedural: flat material colours (glTF baseColorFactor,
emissive on red marks) and an inverted-hull outline; no textures.

Proportions were fitted against the owner's turnaround sheets by orthographic
silhouette overlay (total height 1.34 m including the headset fins).
"""
import math
import os
import sys

import bpy
from mathutils import Quaternion, Vector

TAU = math.tau


def srgb(hex_code):
    out = []
    for i in (0, 2, 4):
        c = int(hex_code[i:i + 2], 16) / 255
        out.append(c / 12.92 if c <= .04045 else ((c + .055) / 1.055) ** 2.4)
    return tuple(out)


PALETTE = {
    "white": "FFFFFF",    # uniform panels, sleeves, gloves, fins
    "dark": "2B2B36",     # outfit structure, headband, collar, belt
    "red": "FF3B47",      # glowing marks and logo (emissive)
    "metal": "D8D8E0",    # buckles and plates
    "skin": "FFE2DD",
    "hair": "F4F4F6",
    "hairshade": "D6D6E2",  # strand bands in long hair (toon shade of the hair colour)
    "boot": "16161D",     # glossy black boots, skirt and shorts
    "eye": "E0283A",      # iris
    "pupil": "5A0E1C",
    "blush": "FFB3B8",
    "outline": "1C1C24",
}
DOUBLE_SIDED = {"hair", "hairshade", "white", "boot"}  # open shells: hair, sleeves, skirt

# Bone layout in metres (Blender: +X is the character's left, -Y is forward).
BONES = {
    "root": ((0, 0, 0), (0, 0, .2), None),
    "hip": ((0, -.02, .655), (0, -.02, .80), "root"),
    "chest": ((0, -.02, .80), (0, -.02, 1.05), "hip"),
    "head": ((0, -.02, 1.05), (0, -.02, 1.30), "chest"),
}
SHOULDER, ELBOW, WRIST = (.072, -.010, 1.02), (.114, .024, .855), (.170, .044, .714)
THIGH, KNEE, ANKLE, TOE = (.052, -.056, .665), (.074, -.008, .375), (.083, .012, .125), (.085, -.11, .015)

HEAD_C = Vector((0, -.032, 1.178))     # face centre
HEAD_R = (.090, .116, .108)            # face radii used to place features


class Builder:
    def __init__(self):
        self.verts, self.weights, self.faces, self.mats, self.smooth = [], [], [], [], []
        self.outlined = []  # (first_face, end_face, thickness)

    # Weight spec: bone name, dict {bone: w}, or callable(point) -> dict.
    def vert(self, p, w):
        p = Vector(p)
        if callable(w):
            w = w(p)
        elif isinstance(w, str):
            w = {w: 1.0}
        self.verts.append(p)
        self.weights.append(w)
        return len(self.verts) - 1

    def face(self, idx, mat, smooth=True):
        self.faces.append(tuple(idx))
        self.mats.append(mat)
        self.smooth.append(smooth)

    def begin(self):
        return len(self.faces)

    def outline(self, start, thickness=.0045):
        self.outlined.append((start, len(self.faces), thickness))

    def loft(self, rings, mat, w, cap_start=True, cap_end=True, smooth=True, mat_fn=None):
        """Rings must run counter-clockwise around the direction of travel (outward normals)."""
        n = len(rings[0])
        ids = [[self.vert(p, w) for p in ring] for ring in rings]
        if cap_start:
            self.face(list(reversed(ids[0])), mat, smooth)
        for r in range(len(rings) - 1):
            a, b = ids[r], ids[r + 1]
            for i in range(n):
                j = (i + 1) % n
                self.face((a[i], a[j], b[j], b[i]), mat_fn(r, i) if mat_fn else mat, smooth)
        if cap_end:
            self.face(ids[-1], mat, smooth)
        return ids

    def sheet(self, grid, mat, w, smooth=True, mat_fn=None):
        """grid[i][k]: i runs left-to-right seen from outside, k runs top-to-bottom."""
        ids = [[self.vert(p, w) for p in col] for col in grid]
        for i in range(len(grid) - 1):
            for k in range(len(grid[0]) - 1):
                self.face((ids[i][k], ids[i][k + 1], ids[i + 1][k + 1], ids[i + 1][k]), mat_fn(i, k) if mat_fn else mat, smooth)
        return ids

    def ellipsoid(self, c, r, mat, w, sides=12, rows=6, smooth=True):
        c = Vector(c)
        rings = []
        for k in range(1, rows):
            a = math.pi * k / rows
            z, s = -math.cos(a), math.sin(a)
            rings.append([c + Vector((r[0] * s * math.cos(TAU * i / sides),
                                      r[1] * s * math.sin(TAU * i / sides), r[2] * z)) for i in range(sides)])
        bottom = self.vert(c - Vector((0, 0, r[2])), w)
        top = self.vert(c + Vector((0, 0, r[2])), w)
        ids = self.loft(rings, mat, w, False, False, smooth)
        for i in range(sides):
            j = (i + 1) % sides
            self.face((bottom, ids[0][j], ids[0][i]), mat, smooth)
            self.face((top, ids[-1][i], ids[-1][j]), mat, smooth)

    def tube(self, a, b, radii, mat, w, sides=10, ts=None, squash=1.0, smooth=True, caps=(True, True), mat_fn=None):
        a, b = Vector(a), Vector(b)
        axis = (b - a).normalized()
        ref = Vector((0, 1, 0)) if abs(axis.y) < .9 else Vector((1, 0, 0))
        u = axis.cross(ref).normalized()
        v = axis.cross(u)
        ts = ts or [k / (len(radii) - 1) for k in range(len(radii))]
        rings = [[a.lerp(b, t) + u * (math.cos(TAU * i / sides) * r) + v * (math.sin(TAU * i / sides) * r * squash)
                  for i in range(sides)] for t, r in zip(ts, radii)]
        return self.loft(rings, mat, w, caps[0], caps[1], smooth, mat_fn)

    def prism(self, outline2d, axis_u, axis_v, origin, depth, mat, w, smooth=False):
        """Extrude a counter-clockwise 2D polygon in the u/v plane along u x v (centred)."""
        o, u, v = Vector(origin), Vector(axis_u), Vector(axis_v)
        n = u.cross(v).normalized()
        front = [o + u * x + v * y - n * depth / 2 for x, y in outline2d]
        back = [o + u * x + v * y + n * depth / 2 for x, y in outline2d]
        self.loft([front, back], mat, w, True, True, smooth)

    def box(self, c, size, mat, w, rot_z=0.0, rot_x=0.0):
        c = Vector(c)
        hx, hy, hz = (s / 2 for s in size)
        q = Quaternion((0, 0, 1), rot_z) @ Quaternion((1, 0, 0), rot_x)
        ring = lambda z: [c + q @ Vector((x, y, z)) for x, y in ((-hx, -hy), (hx, -hy), (hx, hy), (-hx, hy))]
        self.loft([ring(-hz), ring(hz)], mat, w, True, True, False)


def lerp_weights(top_bone, bottom_bone, z_top, z_bottom):
    def f(p):
        t = min(1, max(0, (z_top - p.z) / (z_top - z_bottom)))
        if t <= 0:
            return {top_bone: 1.0}
        if t >= 1:
            return {bottom_bone: 1.0}
        return {top_bone: 1 - t, bottom_bone: t}
    return f


# Long hair follows the head near the scalp and the upper spine further down.
HAIR_W = lerp_weights("head", "chest", 1.10, .90)


def ellipse_ring(z, rx, front, back, cy=0.0, n=16, cx=0.0):
    ring = []
    for i in range(n):
        a = TAU * i / n
        s = math.sin(a)
        ring.append(Vector((cx + rx * math.cos(a), cy + (back if s > 0 else front) * s, z)))
    return ring


def face_y(x, z, lift=.0):
    """Front surface of the face, for placing eyes and marks on it."""
    rx, ry, rz = HEAD_R
    s = 1 - (x / rx) ** 2 - ((z - HEAD_C.z) / rz) ** 2
    return HEAD_C.y - ry * math.sqrt(max(s, 0)) - lift


def build_head(b, long_hair):
    s = b.begin()
    # Face: chin to crown with a soft pointed anime chin.
    prof = [(1.074, .018, .020, -.044), (1.094, .050, .052, -.030), (1.12, .073, .076, -.014),
            (1.16, .088, .093, -.004), (1.20, .090, .097, 0), (1.245, .080, .088, .006), (1.282, .046, .055, .01)]
    rings = [ellipse_ring(z, rx, ry * 1.2, ry * 1.2, HEAD_C.y + dy, 16) for z, rx, ry, dy in prof]
    b.loft(rings, "skin", "head")
    b.tube((0, -.012, 1.02), (0, -.012, 1.10), (.032, .032), "skin", "head", 10)
    b.outline(s, .003)
    s = b.begin()
    b.tube((0, -.006, 1.016), (0, -.006, 1.076), (.046, .044), "dark", "chest", 14, squash=1.12)
    b.outline(s, .003)
    b.box((0, -.058, 1.046), (.012, .006, .018), "red", "chest")  # collar indicator light
    # Eyes: large red irises with dark pupils, highlights and lash line.
    for sx in (-1, 1):
        x, z = sx * .036, 1.160
        y = face_y(x, z)
        b.ellipsoid((x, y + .002, z), (.018, .007, .025), "eye", "head", 10, 5)
        b.ellipsoid((x, y - .004, z - .004), (.010, .003, .014), "pupil", "head", 8, 4)
        b.ellipsoid((x - sx * .006, y - .007, z + .010), (.0055, .002, .0055), "white", "head", 6, 3)
        b.box((x + sx * .002, face_y(x, z + .027, .002), z + .027), (.044, .007, .007), "dark", "head")
        b.box((x + sx * .024, face_y(x + sx * .024, z + .019, .001), z + .019), (.011, .006, .006), "dark", "head", sx * -.5)
        yb = face_y(sx * .056, 1.126)
        b.ellipsoid((sx * .056, yb + .003, 1.126), (.013, .003, .006), "blush", "head", 8, 3)
    b.box((0, face_y(0, 1.104, .001), 1.104), (.012, .004, .003), "pupil", "head")  # small mouth

    # Hair cap with side-swept bangs: grid over (angle, height), open at the face.
    s = b.begin()
    cols, rows = 28, 8
    cz, rx, ry, rz, cy = 1.185, .114, .158, .122, -.026
    grid = []
    for i in range(cols + 1):
        th = TAU * i / cols  # 0 = front (-Y), increasing toward +X: left-to-right seen from outside
        front, side = math.cos(th), math.sin(th)
        jag = .014 if i % 2 else 0
        if front > .86:  # bangs, swept toward the character's right (-X)
            bottom = 1.186 - .024 * max(0, -side) + jag
        elif front > .5:  # face-framing locks down to the jaw
            bottom = 1.105 + jag
        elif front > .05:
            bottom = 1.115 + jag
        else:
            bottom = (1.074 if not long_hair else 1.06) + jag
        col = []
        for k in range(rows):
            t = k / (rows - 1)
            z = cz + rz - t * (cz + rz - bottom)
            if z >= cz:
                sc = math.sqrt(max(0, 1 - ((z - cz) / rz) ** 2))
            else:
                sc = 1 + (.09 + (.0 if not long_hair else .06) * max(0, -front)) * (cz - z) / (cz - 1.06)
            sc = max(sc, .03)
            p = Vector((rx * sc * side, cy - ry * sc * front, z))
            if front > .5 and z < cz + .05:
                lift = .010 if front > .86 else .028
                # Bangs curl back onto the forehead instead of floating in front of it.
                target = face_y(max(-HEAD_R[0] * .95, min(HEAD_R[0] * .95, p.x)), z, lift)
                blend = min(1, (cz + .05 - z) / .065)
                p.y = p.y + (target - p.y) * blend
            col.append(p)
        grid.append(col)
    b.sheet(grid, "hair", "head")
    b.outline(s, .004)
    b.box((.052, face_y(.052, 1.236, .040), 1.236), (.028, .008, .011), "dark", "head", .15, -.5)  # hair clip
    b.box((.052, face_y(.052, 1.236, .045), 1.236), (.018, .003, .004), "metal", "head", .15, -.5)

    # Headset: dark headband, white cups with black face and red chevron, fins.
    s = b.begin()
    hc = Vector((0, -.012, 1.170))
    rings = []
    for k in range(13):
        a = math.radians(-74 + 148 * k / 12)
        p = hc + Vector((.150 * math.sin(a), 0, .142 * math.cos(a)))
        out = (p - hc).normalized()
        rings.append([p + out * dx + Vector((0, dy, 0)) for dx, dy in ((-.006, -.014), (-.006, .014), (.006, .014), (.006, -.014))])
    b.loft(rings, "dark", "head", smooth=False)
    b.outline(s, .003)
    for sx in (-1, 1):
        s = b.begin()
        cup = Vector((sx * .124, -.012, 1.165))
        b.tube(cup - Vector((sx * .016, 0, 0)), cup + Vector((sx * .022, 0, 0)), (.040, .046, .046), "white", "head", 12, (0, .5, 1))
        b.tube(cup + Vector((sx * .022, 0, 0)), cup + Vector((sx * .026, 0, 0)), (.036, .034), "dark", "head", 12, smooth=False)
        b.outline(s, .004)
        s = b.begin()
        # Fin: angular plate rising up and back from the top of the cup.
        fin = [(-.022, 0), (.032, -.010), (.120, .134), (.098, .140), (.018, .056)]
        base = cup + Vector((-sx * .002, 0, .036))
        tilt = Vector((-sx * .18, 0, 1)).normalized()
        b.prism(fin, (0, 1, 0), tilt, base, .012, "white", "head")
        b.outline(s, .003)
        # Red chevron "^" on the black face.
        face_x = cup.x + sx * .0275
        for d in (-1, 1):
            b.box((face_x, cup.y + d * .0075, cup.z), (.004, .007, .030), "red", "head", 0, d * .5)

    if long_hair:
        build_long_hair(b)


def build_long_hair(b):
    # Front side locks falling in front of the shoulders (rings ascend so normals face out).
    s = b.begin()
    for sx in (-1, 1):
        path = [(sx * .118, -.050, .80), (sx * .116, -.066, .88), (sx * .110, -.078, .98),
                (sx * .100, -.082, 1.07), (sx * .092, -.080, 1.16)]
        widths = [.006, .024, .030, .028, .026]
        rings = [[Vector(p) + Vector((wdt * math.cos(TAU * i / 8), .012 * math.sin(TAU * i / 8), 0)) for i in range(8)]
                 for p, wdt in zip(path, widths)]
        b.loft(rings, "hair", HAIR_W)
    b.outline(s, .003)

    def layer(profile, span, cols, jag, cy=.0):
        """profile: (z, rx, ry) from the crown down; columns run left-to-right seen from behind."""
        s = b.begin()
        grid = []
        for i in range(cols):
            u = span - 2 * span * i / (cols - 1)  # +X side first: left-to-right from behind
            tip = jag * (1, .25, .65)[i % 3] + jag * .8 * (abs(u) / span) ** 2
            ridge = 1 + (.06 if i % 2 else -.03)
            col = []
            for k, (z, rx, ry) in enumerate(profile):
                if k == len(profile) - 1:
                    z += tip
                rr = ridge if k else 1
                c = math.cos(u)
                depth = math.copysign(abs(c) ** .6, c)  # flatter back, fuller sides: clears the arms
                col.append(Vector((rr * rx * math.sin(u), cy + rr * ry * depth, z)))
            grid.append(col)
        b.sheet(grid, "hair", HAIR_W, False, lambda i, k: "hairshade" if i % 3 == 1 and k > 1 else "hair")
        b.outline(s, .004)

    # Lower layer to the thighs, then the upper layer over it ending at the waist.
    layer([(1.20, .104, .136), (1.08, .118, .140), (.98, .150, .168), (.90, .200, .196), (.80, .214, .204),
           (.70, .210, .185), (.62, .180, .150), (.545, .150, .110)], math.radians(97), 15, .075)
    layer([(1.20, .106, .140), (1.10, .126, .158), (1.0, .160, .176), (.92, .206, .208), (.82, .220, .214),
           (.76, .212, .208)], math.radians(82), 13, .06)


def build_body(b):
    # Torso: dark bodysuit with a white central panel; sides and back fully covered.
    s = b.begin()
    prof = [(.640, .056, .060, .050), (.700, .086, .074, .062), (.780, .086, .072, .062), (.845, .062, .068, .046),
            (.900, .066, .078, .058), (.950, .074, .094, .070), (.990, .076, .088, .072), (1.025, .064, .066, .060),
            (1.055, .042, .046, .040)]
    sides = 20
    rings = [ellipse_ring(z, rx, df, db, -.022, sides) for z, rx, df, db in prof]

    def panel(row, i):
        centre = math.degrees(TAU * (i + .5) / sides)
        half = (0, 0, 0, 16, 16, 31, 31, 31)[row]
        return "white" if abs(centre - 270) < half else "dark"
    w = lambda p: {"hip": 1.0} if p.z < .80 else {"chest": 1.0}
    b.loft(rings, "dark", w, mat_fn=panel)
    b.outline(s)
    # Chest logo: small red triangle on the white panel.
    b.prism([(-.012, -.008), (.012, -.008), (0, .013)], (1, 0, 0), (0, 0, 1), (0, -.022 - .095, .975), .004, "red", "chest")
    # Belt with silver square buckle.
    s = b.begin()
    b.loft([ellipse_ring(z, .068, .074, .052, -.022, 20) for z in (.832, .866)], "dark", "hip", False, False)
    b.outline(s, .003)
    b.box((0, -.098, .849), (.036, .006, .030), "metal", "hip")
    b.box((0, -.1015, .849), (.020, .004, .016), "dark", "hip")
    # Pleated mini skirt: black outer pleats, white and red inner folds at the sides and back.
    s = b.begin()
    pleats = 18
    n = pleats * 2
    rings = []
    for z, rx, ry, depth in ((.660, .132, .150, .12), (.765, .114, .130, .08), (.848, .072, .074, .01)):
        ring = []
        for i in range(n):
            a = TAU * i / n
            k = 1 - (depth if i % 2 else 0)
            lift = (.045 if z < .7 else .02 if z < .8 else 0) * max(0, -math.sin(a))
            ring.append(Vector((rx * k * math.cos(a), -.012 + ry * k * math.sin(a) * (1.12 if math.sin(a) < 0 else 1), z + lift)))
        rings.append(ring)

    def pleat(row, i):
        frontness = -math.sin(TAU * (i + .5) / n)
        if i % 2 == 1 and frontness < .55:
            return "red" if (i // 2) % 3 == 0 else "white"
        return "boot"
    b.loft(rings, "boot", "hip", False, False, False, pleat)
    b.outline(s, .004)
    # Dark shorts under the skirt, hugging each thigh.
    s = b.begin()
    for sx in (-1, 1):
        b.tube((sx * .050, -.042, .700), (sx * .053, -.038, .640), (.062, .059, .057), "boot", "hip", 12)
    b.outline(s, .003)


def build_arms(b):
    for side, sx in (("L", 1), ("R", -1)):
        sh = Vector((sx * SHOULDER[0], SHOULDER[1], SHOULDER[2]))
        el = Vector((sx * ELBOW[0], ELBOW[1], ELBOW[2]))
        wr = Vector((sx * WRIST[0], WRIST[1], WRIST[2]))
        up, fo = "upper." + side, "fore." + side
        s = b.begin()
        b.ellipsoid(sh + Vector((0, 0, -.004)), (.034, .034, .033), "skin", up, 10, 5)
        b.tube(sh, sh.lerp(el, .42), (.031, .030), "skin", up, 10, caps=(False, False))
        # Detached sleeve: dark top band, white sleeve flaring to a dark cuff with a red stripe.
        b.tube(sh.lerp(el, .34), sh.lerp(el, .44), (.037, .038), "dark", up, 10, caps=(False, False))
        b.tube(sh.lerp(el, .44), el, (.038, .041), "white", up, 10, caps=(False, False))
        b.tube(el, wr, (.041, .044, .049, .054), "white", fo, 10, (0, .4, .75, .82), caps=(False, False))
        b.tube(el.lerp(wr, .82), el.lerp(wr, .88), (.0545, .055), "red", fo, 10, caps=(False, False))
        b.tube(el.lerp(wr, .88), wr + (wr - el).normalized() * .012, (.056, .058), "dark", fo, 10, caps=(False, False))
        b.tube(el, wr, (.026, .022), "skin", fo, 8, caps=(False, False))
        # Glove: white hand with thumb.
        hand = wr + Vector((sx * .002, -.004, -.044))
        b.ellipsoid(hand, (.016, .029, .043), "white", fo, 10, 5)
        b.ellipsoid(hand + Vector((-sx * .010, -.019, .014)), (.009, .010, .019), "white", fo, 6, 4)
        b.outline(s, .0035)
        b.box(el.lerp(wr, .5) + Vector((sx * .047, 0, 0)), (.004, .008, .07), "red", fo)  # outer stripe


def build_legs(b):
    for side, sx in (("L", 1), ("R", -1)):
        th = Vector((sx * THIGH[0], THIGH[1], THIGH[2]))
        kn = Vector((sx * KNEE[0], KNEE[1], KNEE[2]))
        an = Vector((sx * ANKLE[0], ANKLE[1], ANKLE[2]))
        tg, sg, fg = "thigh." + side, "shin." + side, "foot." + side
        s = b.begin()
        b.tube(th, th.lerp(kn, .24), (.049, .047), "skin", tg, 10, caps=(False, False))
        # Thigh-high glossy boot.
        b.tube(th.lerp(kn, .22), kn, (.047, .044, .041, .046), "boot", tg, 10, (0, .1, .6, 1), squash=1.22, caps=(True, False))
        b.tube(kn, an, (.048, .050, .031, .025), "boot", sg, 10, (0, .22, .75, 1), squash=1.2, caps=(False, False))
        # Boot foot with a low block heel.
        foot = []
        for y, half_w, z0, z1 in ((.088, .018, .075, .150), (.050, .028, .062, .150), (-.025, .033, .026, .138),
                                  (-.060, .032, .006, .066), (-.105, .025, .001, .038), (-.132, .012, .002, .018)):
            foot.append([Vector((an.x + half_w * math.cos(TAU * i / 8), y + an.y - .022, (z0 + z1) / 2 + (z1 - z0) / 2 * math.sin(TAU * i / 8)))
                         for i in range(8)])
        b.loft(foot, "boot", fg)
        b.box((an.x, an.y + .052, .036), (.022, .022, .072), "boot", fg)
        b.outline(s, .004)
        # Knee armour plate with red triangle.
        s = b.begin()
        plate = [(-.026, -.052), (.026, -.052), (.033, .010), (0, .064), (-.033, .010)]
        kc = kn + Vector((0, -.060, -.006))
        b.prism(plate, (1, 0, 0), (0, 0, 1), kc, .012, "white", sg)
        b.outline(s, .003)
        b.prism([(-.008, -.006), (.008, -.006), (0, .009)], (1, 0, 0), (0, 0, 1), kc + Vector((0, -.008, -.014)), .004, "red", sg)
        # Outer shin guard.
        s = b.begin()
        guard = [(-.014, -.07), (.012, -.06), (.012, .06), (-.004, .08)]
        b.prism(guard, (0, 1, 0) if sx > 0 else (0, -1, 0), (0, 0, 1), kn.lerp(an, .26) + Vector((sx * .046, 0, 0)), .008, "white", sg)
        b.outline(s, .002)
    # One dark leg strap with buckle on the left thigh at the boot top.
    c = Vector(THIGH).lerp(Vector(KNEE), .24)
    s = b.begin()
    b.tube(c + Vector((0, 0, .012)), c - Vector((0, 0, .012)), (.061, .061), "dark", "thigh.L", 14, caps=(False, False))
    b.outline(s, .002)
    b.box((c.x + .036, c.y - .050, c.z), (.024, .008, .024), "metal", "thigh.L", .7)
    b.box((c.x + .037, c.y - .052, c.z), (.012, .006, .012), "dark", "thigh.L", .7)
    b.box((c.x + .062, c.y - .004, c.z - .006), (.012, .030, .036), "dark", "thigh.L")  # small pouch
    b.box((c.x + .069, c.y - .004, c.z - .002), (.002, .014, .004), "red", "thigh.L")


def add_outline(b):
    """Inverted hull: offset copies of outlined faces, reversed and drawn single-sided."""
    normals = [Vector() for _ in b.verts]
    for start, end, _t in b.outlined:
        for f in b.faces[start:end]:
            pts = [b.verts[i] for i in f]
            n = Vector()
            for k in range(len(pts)):
                a, c = pts[k], pts[(k + 1) % len(pts)]
                n += Vector(((a.y - c.y) * (a.z + c.z), (a.z - c.z) * (a.x + c.x), (a.x - c.x) * (a.y + c.y)))
            for i in f:
                normals[i] += n
    for start, end, t in b.outlined:
        remap = {}
        for f in b.faces[start:end]:
            new = []
            for i in f:
                if i not in remap:
                    n = normals[i].normalized() if normals[i].length > 1e-12 else Vector()
                    remap[i] = b.vert(b.verts[i] + n * t, dict(b.weights[i]))
                new.append(remap[i])
            b.faces.append(tuple(reversed(new)))
            b.mats.append("outline")
            b.smooth.append(False)


def clear_scene():
    if bpy.context.object and bpy.context.object.mode != "OBJECT":
        bpy.ops.object.mode_set(mode="OBJECT")
    for obj in list(bpy.data.objects):
        bpy.data.objects.remove(obj, do_unlink=True)
    for coll in (bpy.data.meshes, bpy.data.armatures, bpy.data.materials, bpy.data.actions,
                 bpy.data.cameras, bpy.data.lights):
        for item in list(coll):
            coll.remove(item)


def make_material(name):
    mat = bpy.data.materials.new(name)
    try:
        mat.use_nodes = True
    except AttributeError:
        pass
    bsdf = mat.node_tree.nodes.get("Principled BSDF")
    color = srgb(PALETTE[name])
    bsdf.inputs["Base Color"].default_value = (*color, 1)
    bsdf.inputs["Roughness"].default_value = .25 if name == "boot" else .8
    bsdf.inputs["Metallic"].default_value = 0.0
    if name == "red":
        bsdf.inputs["Emission Color"].default_value = (*color, 1)
        bsdf.inputs["Emission Strength"].default_value = 1.0
    mat.diffuse_color = (*color, 1)
    mat.use_backface_culling = name not in DOUBLE_SIDED
    return mat


def build(variant):
    clear_scene()
    armature = bpy.data.armatures.new("Axiom mascot skeleton")
    rig = bpy.data.objects.new("Axiom", armature)
    bpy.context.collection.objects.link(rig)
    bpy.context.view_layer.objects.active = rig
    rig.select_set(True)
    bpy.ops.object.mode_set(mode="EDIT")

    def bone(name, head, tail, parent=None):
        eb = armature.edit_bones.new(name)
        eb.head, eb.tail = head, tail
        if parent:
            eb.parent = armature.edit_bones[parent]
    for name, (h, t, p) in BONES.items():
        bone(name, h, t, p)
    for side, sx in (("L", 1), ("R", -1)):
        m = lambda v: (sx * v[0], v[1], v[2])
        bone("upper." + side, m(SHOULDER), m(ELBOW), "chest")
        bone("fore." + side, m(ELBOW), m(WRIST), "upper." + side)
        bone("thigh." + side, m(THIGH), m(KNEE), "hip")
        bone("shin." + side, m(KNEE), m(ANKLE), "thigh." + side)
        bone("foot." + side, m(ANKLE), m(TOE), "shin." + side)
    bpy.ops.object.mode_set(mode="OBJECT")

    b = Builder()
    build_head(b, variant == "long")
    build_body(b)
    build_arms(b)
    build_legs(b)
    add_outline(b)

    names = list(PALETTE)
    mesh = bpy.data.meshes.new("Axiom mascot " + variant)
    mesh.from_pydata([tuple(v) for v in b.verts], [], b.faces)
    mesh.validate()
    obj = bpy.data.objects.new("Body", mesh)
    bpy.context.collection.objects.link(obj)
    obj.parent = rig
    used = sorted(set(b.mats), key=names.index)
    for name in used:
        mesh.materials.append(make_material(name))
    for poly, mat, smooth in zip(mesh.polygons, b.mats, b.smooth):
        poly.material_index = used.index(mat)
        poly.use_smooth = smooth
    groups = {name: obj.vertex_groups.new(name=name) for name in rig.data.bones.keys()}
    for i, w in enumerate(b.weights):
        total = sum(w.values())
        for name, value in w.items():
            groups[name].add([i], value / total, "REPLACE")
    modifier = obj.modifiers.new("Skeleton", "ARMATURE")
    modifier.object = rig
    add_clips(rig)
    return rig, obj


def add_clips(rig):
    """Same five clips and pose tables as scripts/blender-duet.py (mirror = 1)."""
    scene = bpy.context.scene
    scene.render.fps = 30
    scene.frame_start, scene.frame_end = 1, 25
    rig.animation_data_create()
    mirror = 1
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
                angles["upper.L"] = (-66 * w, -35 * w * mirror, 0)
            if name == "finale":
                angles["upper.L"] = (-25 * w, -115 * w, -30 * w)
                angles["upper.R"] = (-28 * w, 110 * w, 28 * w)
            for pose in rig.pose.bones:
                pose.rotation_mode = "QUATERNION"
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
    for pose in rig.pose.bones:
        pose.rotation_quaternion = Quaternion()
    scene.frame_set(1)


def export(rig, obj, path):
    bpy.ops.object.select_all(action="DESELECT")
    rig.select_set(True)
    obj.select_set(True)
    bpy.context.view_layer.objects.active = rig
    bpy.ops.export_scene.gltf(filepath=path, export_format="GLB", use_selection=True,
        export_animations=True, export_frame_range=True, export_force_sampling=True,
        export_animation_mode="ACTIONS", export_anim_single_armature=True, export_skins=True)


def triangle_count(obj):
    return sum(len(p.vertices) - 2 for p in obj.data.polygons)


if __name__ == "__main__" and "--" in sys.argv:
    out = os.path.abspath(sys.argv[sys.argv.index("--") + 1])
    os.makedirs(out, exist_ok=True)
    for variant in ("short", "long"):
        rig, obj = build(variant)
        path = os.path.join(out, "axiom-%s.glb" % variant)
        export(rig, obj, path)
        print("MASCOT %s triangles=%d bytes=%d" % (variant, triangle_count(obj), os.path.getsize(path)))
