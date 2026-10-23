#!/usr/bin/env python3
"""Builds the example that ships inside Tapetum (crates/tapetum/assets/example/de|en/):

  Tapetum-Demo.igz      a small theatre as an IngeTrazo document (format 2, a ZIP
                        with document.json + textures/), with the pergola bench of
                        crates/tapetum/tests/data and its face-me figure, four times
                        (CC BY 4.0, Marco Sumari Tellez)
  Tapetum-Demo.olga     a BlinderKitten show: the fixtures of the test show, hung in
                        that theatre, with a short show of cues
  Tapetum-Demo.tapetum  Tapetum's project: the .igz, fixture settings, scenes with
                        set changes bound to cues

German and English differ only in the names. The theatre is modelled here;
nothing is downloaded. Standard library only; the show gets its Tapetum mirror
interfaces from Tapetum itself (`tapetum mirror --offline`, via cargo), so it
works without the setup step.
Run after changing it:

    python3 scripts/gen-example.py
"""
import copy
import io
import json
import math
import subprocess
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "crates/tapetum/assets/example"
NAME = "Tapetum-Demo"

# ── Geometry (IngeTrazo: metres, Z up, the audience looks along +Y) ──────────
# Faces are counterclockwise seen from outside, colours (r, g, b, 1) in 0..1.

STAGE_FRONT, STAGE_BACK = -4.4, 7.2

NAMES = {
    "de": {
        "house": "Zuschauerraum Boden", "seats": "Zuschauerreihen", "stage": "Bühne mit Seitenbühnen", "cyc": "Rundhorizont",
        "portal": "Portal", "curtain": "Vorhang", "masking": "Gassen und Soffitten", "truss": "Traversen",
        "podest": "Podest", "columns": "Säulen", "table": "Tisch", "chairs": "Stühle", "lantern": "Laterne",
        "crates": "Kisten", "bench": "Bank", "person": "Person",
        "fixtures": {31: "Seite L", 32: "Seite R", 41: "Wash L", 42: "Wash R", 51: "Laterne", 61: "Front 1", 62: "Front 2", 63: "Front 3", 64: "Front 4"},
        "groups": {3: "Seite", 4: "Wash", 5: "Laterne", 6: "Front"}, "bridge": "Frontlichtbrücke",
        "cues": ["Einlass", "Prolog", "Abend", "Nacht", "Morgen", "Fest", "Finale", "Blackout"],
        "chapters": ["1 - Einlass", "", "2 - Abend", "", "3 - Morgen", "4 - Fest", "5 - Finale", ""],
        "scenes": ["Morgen", "Fest", "Finale"],
    },
    "en": {
        "house": "Auditorium floor", "seats": "Rows of seats", "stage": "Stage with side stages", "cyc": "Cyclorama",
        "portal": "Proscenium", "curtain": "Curtain", "masking": "Legs and borders", "truss": "Trusses",
        "podest": "Platform", "columns": "Columns", "table": "Table", "chairs": "Chairs", "lantern": "Lantern",
        "crates": "Crates", "bench": "Bench", "person": "Person",
        "fixtures": {31: "Side L", 32: "Side R", 41: "Wash L", 42: "Wash R", 51: "Lantern", 61: "Front 1", 62: "Front 2", 63: "Front 3", 64: "Front 4"},
        "groups": {3: "Side", 4: "Wash", 5: "Lantern", 6: "Front"}, "bridge": "Front light bridge",
        "cues": ["Pre-show", "Prologue", "Evening", "Night", "Morning", "Party", "Finale", "Blackout"],
        "chapters": ["1 - Pre-show", "", "2 - Evening", "", "3 - Morning", "4 - Party", "5 - Finale", ""],
        "scenes": ["Morning", "Party", "Finale"],
    },
}


class Group:
    def __init__(self, name, uid, color):
        self.name, self.uid, self.color, self.faces = name, uid, color, []

    def face(self, *pts):
        self.faces.append([tuple(round(c, 4) for c in p) for p in pts])

    def box(self, x0, x1, y0, y1, z0, z1):
        self.face((x0, y0, z0), (x1, y0, z0), (x1, y0, z1), (x0, y0, z1))
        self.face((x1, y1, z0), (x0, y1, z0), (x0, y1, z1), (x1, y1, z1))
        self.face((x0, y1, z0), (x0, y0, z0), (x0, y0, z1), (x0, y1, z1))
        self.face((x1, y0, z0), (x1, y1, z0), (x1, y1, z1), (x1, y0, z1))
        self.face((x0, y0, z1), (x1, y0, z1), (x1, y1, z1), (x0, y1, z1))
        self.face((x0, y1, z0), (x1, y1, z0), (x1, y0, z0), (x0, y0, z0))

    def cylinder(self, cx, cy, r, z0, z1, n=20):
        ring = [(cx + r * math.cos(2 * math.pi * i / n), cy + r * math.sin(2 * math.pi * i / n)) for i in range(n)]
        for i in range(n):
            (ax, ay), (bx, by) = ring[i], ring[(i + 1) % n]
            self.face((ax, ay, z0), (bx, by, z0), (bx, by, z1), (ax, ay, z1))
        self.face(*[(x, y, z1) for x, y in ring])
        self.face(*[(x, y, z0) for x, y in reversed(ring)])

    def json(self):
        edges, seen = [], set()
        for f in self.faces:
            for a, b in zip(f, f[1:] + f[:1]):
                k = (min(a, b), max(a, b))
                if k not in seen:
                    seen.add(k)
                    edges.append({"a": list(a), "b": list(b)})
        return {"name": self.name, "uid": self.uid, "component": False, "edges": edges,
                "faces": [{"vertices": [list(p) for p in f], "color": list(self.color)} for f in self.faces]}


def drape(g, xout, xin_top, xin_tie, xin_bot, y, z0, z1, tie, folds=9, depth=0.13, n=60, m=14):
    """A curtain gathered towards `xout`, its inner edge pulled to a tie-back. Triangles: folds are not planar."""
    def xin(z):
        if z > tie:
            return xin_tie + (xin_top - xin_tie) * ((z - tie) / (z1 - tie)) ** 1.4
        return xin_tie + (xin_bot - xin_tie) * ((tie - z) / (tie - z0)) ** 0.7
    rows = []
    for j in range(m + 1):
        z = z0 + (z1 - z0) * j / m
        xi = xin(z)
        gather = abs(xi - xout) / abs(xin_top - xout)
        rows.append([(xout + (xi - xout) * i / n, y + depth * math.sin(i / n * folds * 2 * math.pi) * (1.25 - 0.5 * gather), z) for i in range(n + 1)])
    flip = xin_top < xout
    for j in range(m):
        for i in range(n):
            a, b, c, d = rows[j][i], rows[j][i + 1], rows[j + 1][i + 1], rows[j + 1][i]
            for t in ((a, b, c), (a, c, d)):
                g.face(*(t[::-1] if flip else t))


def valance(g, x0, x1, y, z0, z1, folds=40, depth=0.08, n=240):
    for i in range(n):
        xa, xb = x0 + (x1 - x0) * i / n, x0 + (x1 - x0) * (i + 1) / n
        ya, yb = (y + depth * math.sin(k / n * folds * 2 * math.pi) for k in (i, i + 1))
        g.face((xa, ya, z0), (xb, yb, z0), (xb, yb, z1))
        g.face((xa, ya, z0), (xb, yb, z1), (xa, ya, z1))


def theatre(n):
    G = []
    def group(key, uid, rgb):
        g = Group(n[key], uid, (*rgb, 1.0))
        G.append(g)
        return g

    house = group("house", "demo-house", (0.05, 0.04, 0.04))
    house.box(-16, 16, -18, STAGE_FRONT, -1.02, -1.0)
    seats = group("seats", "demo-seats", (0.28, 0.03, 0.04))
    for r in range(7):
        y, z = -7.2 - r * 1.05, -1.0 + r * 0.16
        if r:
            house.box(-16, 16, y - 0.6, y + 0.45, z - 0.02, z)
        for k in range(-12, 13):
            if k:
                x = k * 0.56
                seats.box(x - 0.24, x + 0.24, y - 0.59, y - 0.5, z + 0.42, z + 1.0)
                seats.box(x - 0.24, x + 0.24, y - 0.5, y, z + 0.38, z + 0.5)
    group("stage", "demo-stage", (0.30, 0.28, 0.27)).box(-12, 12, STAGE_FRONT, STAGE_BACK, -1.0, 0.0)
    group("cyc", "demo-cyc", (0.82, 0.82, 0.85)).face((-9, 6.9, 0), (9, 6.9, 0), (9, 6.9, 9), (-9, 6.9, 9))
    portal = group("portal", "demo-portal", (0.09, 0.02, 0.025))
    portal.box(-11, -6.4, -5.0, -4.6, -1, 9.5)
    portal.box(6.4, 11, -5.0, -4.6, -1, 9.5)
    portal.box(-6.4, 6.4, -5.0, -4.6, 6.6, 9.5)
    curtain = group("curtain", "demo-curtain", (0.55, 0.035, 0.05))
    drape(curtain, -6.4, -4.3, -5.35, -4.95, -4.45, 0, 6.6, 2.2)
    drape(curtain, 6.4, 4.3, 5.35, 4.95, -4.45, 0, 6.6, 2.2)
    valance(curtain, -6.4, 6.4, -4.4, 5.9, 6.6)
    masking = group("masking", "demo-masking", (0.025, 0.025, 0.025))
    for y in (0.0, 3.4):
        for s in (-1, 1):
            masking.box(min(s * 5.3, s * 7.6), max(s * 5.3, s * 7.6), y, y + 0.05, 0, 8)
        masking.box(-7.6, 7.6, y - 0.4, y - 0.35, 7.4, 8.6)
    truss = group("truss", "demo-truss", (0.5, 0.5, 0.52))
    for y in (-1.6, 1.8):
        for dy in (-0.15, 0.15):
            for dz in (-0.15, 0.15):
                truss.box(-6.5, 6.5, y + dy - 0.02, y + dy + 0.02, 7.05 + dz - 0.02, 7.05 + dz + 0.02)

    # The front light bridge over the auditorium.
    bridge = group("bridge", "demo-bridge", (0.5, 0.5, 0.52))
    for dy in (-0.15, 0.15):
        for dz in (-0.15, 0.15):
            bridge.box(-5.0, 5.0, FRONT_Y + dy - 0.02, FRONT_Y + dy + 0.02, FRONT_Z + 0.35 + dz - 0.02, FRONT_Z + 0.35 + dz + 0.02)

    podest = group("podest", "demo-podest", (0.22, 0.2, 0.22))
    # Clear of the leg at y = 3.4 (a shared face flickers).
    podest.box(-5.2, -2.4, 3.5, 5.8, 0, 0.8)
    for k in range(3):
        podest.box(-4.6, -3.2, 3.5 - 0.32 * (3 - k), 3.5 - 0.32 * (2 - k), 0, 0.2 * (k + 1))
    columns = group("columns", "demo-columns", (0.78, 0.74, 0.68))
    for cx, cy in ((3.4, 4.9), (5.0, 4.3)):
        columns.box(cx - 0.36, cx + 0.36, cy - 0.36, cy + 0.36, 0, 0.25)
        columns.cylinder(cx, cy, 0.24, 0.25, 3.9)
        columns.box(cx - 0.36, cx + 0.36, cy - 0.36, cy + 0.36, 3.9, 4.15)
    columns.box(3.0, 5.4, 4.0, 5.2, 4.15, 4.45)
    tx, ty = 3.1, -1.4
    table = group("table", "demo-table", (0.42, 0.25, 0.13))
    table.box(tx - 0.55, tx + 0.55, ty - 0.38, ty + 0.38, 0.72, 0.77)
    for sx in (-0.48, 0.48):
        for sy in (-0.31, 0.31):
            table.box(tx + sx - 0.03, tx + sx + 0.03, ty + sy - 0.03, ty + sy + 0.03, 0, 0.72)
    chairs = group("chairs", "demo-chairs", (0.42, 0.25, 0.13))
    for side in (-1, 1):
        cx = tx + side * 0.95
        chairs.box(cx - 0.22, cx + 0.22, ty - 0.22, ty + 0.22, 0.43, 0.47)
        for sx in (-0.19, 0.19):
            for sy in (-0.19, 0.19):
                chairs.box(cx + sx - 0.02, cx + sx + 0.02, ty + sy - 0.02, ty + sy + 0.02, 0, 0.43)
        bx = cx + side * 0.2
        chairs.box(bx - 0.02, bx + 0.02, ty - 0.22, ty + 0.22, 0.47, 0.95)
    lx, ly = LANTERN
    lantern = group("lantern", "demo-lantern", (0.08, 0.08, 0.09))
    lantern.box(lx - 0.15, lx + 0.15, ly - 0.15, ly + 0.15, 0, 0.12)
    lantern.cylinder(lx, ly, 0.045, 0.12, 2.85, 10)
    lantern.box(lx - 0.17, lx + 0.17, ly - 0.17, ly + 0.17, 3.2, 3.26)
    lantern.box(lx - 0.13, lx + 0.13, ly - 0.13, ly + 0.13, 2.83, 2.87)
    for sx in (-0.13, 0.13):
        for sy in (-0.13, 0.13):
            lantern.box(lx + sx - 0.012, lx + sx + 0.012, ly + sy - 0.012, ly + sy + 0.012, 2.87, 3.2)
    crates = group("crates", "demo-crates", (0.5, 0.37, 0.2))
    for x, y, z, a in ((-5.0, 0.9, 0, 0.7), (-4.2, 1.1, 0, 0.6), (-4.65, 1.0, 0.7, 0.55), (-5.3, 2.0, 0, 0.5)):
        crates.box(x - a / 2, x + a / 2, y - a / 2, y + a / 2, z, z + a)
    return G


LANTERN = (-3.3, -1.9)
# Where the front light hangs: a bridge over the rows of seats.
FRONT_Y, FRONT_Z = -10.5, 4.6
# Four on the bridge, each lighting its own quarter of the stage width; their
# pools overlap, so the stage is lit evenly.
FRONT = {61: (-4.2, -3.3), 62: (-1.4, -1.1), 63: (1.4, 1.1), 64: (4.2, 3.3)}


# The bench faces stage right in its file: turned a quarter to face the audience,
# and lifted 2 cm so its foundations no longer lie in the stage floor's plane.
BENCH_TURN, BENCH_LIFT = -90.0, 0.02


def bench_transform(groups):
    pts = [v for g in groups for f in g.get("faces", []) for v in f["vertices"]]
    cx = (min(p[0] for p in pts) + max(p[0] for p in pts)) / 2
    cy = (min(p[1] for p in pts) + max(p[1] for p in pts)) / 2
    c, s_ = math.cos(math.radians(BENCH_TURN)), math.sin(math.radians(BENCH_TURN))
    # p' = R p + t, R about Z through the centre, then lifted.
    t = (cx - (c * cx - s_ * cy), cy - (s_ * cx + c * cy), BENCH_LIFT)
    point = lambda p: [c * p[0] - s_ * p[1] + t[0], s_ * p[0] + c * p[1] + t[1], p[2] + t[2]]

    def mesh(m):
        for e in m.get("edges", []):
            e["a"], e["b"] = point(e["a"]), point(e["b"])
        for f in m.get("faces", []):
            f["vertices"] = [point(v) for v in f["vertices"]]
            if "holes" in f:
                f["holes"] = [[point(v) for v in h] for h in f["holes"]]
    for g in groups:
        if "xform" in g:
            # Column-major 4x4: rotate its axes and its origin.
            x = g["xform"]
            for col in range(3):
                vx, vy = x[col * 4], x[col * 4 + 1]
                x[col * 4], x[col * 4 + 1] = c * vx - s_ * vy, s_ * vx + c * vy
            x[12], x[13], x[14] = point(x[12:15])
        else:
            mesh(g)


# Where the bench model's face-me figure stands, then three more of it (its
# picture is part of that CC BY document); the first one is the model's own.
PEOPLE = ((-0.65, -0.6), (2.4, -0.5), (-2.0, 0.7), (4.3, 3.1))


def figure(name, uid, embed, w, h, x, y, z):
    """A face-me figure as IngeTrazo makes it: one quad its picture covers."""
    quad = [[x - w / 2, y, z], [x + w / 2, y, z], [x + w / 2, y, z + h], [x - w / 2, y, z + h]]
    edges = [{"a": quad[i], "b": quad[(i + 1) % 4]} for i in range(4)]
    return {"name": name, "uid": uid, "edges": edges, "billboard": True,
            "faces": [{"vertices": quad, "texture": {"sw": w, "sh": h, "embed": embed}}]}


def write_igz(lang):
    n = NAMES[lang]
    bench = ROOT / "crates/tapetum/tests/data/banca-pergola-yanque.igz"
    with zipfile.ZipFile(bench) as z:
        doc = json.loads(z.read("document.json"))
        textures = {k: z.read(k) for k in z.namelist() if k.startswith("textures/")}
    doc["app_version"] = "0.5.7"
    scene = doc["scene"]
    # The bench's own groups become one named group, the face-me figure stays apart.
    person = [g for g in scene["groups"] if g.get("billboard")]
    rest = [g for g in scene["groups"] if not g.get("billboard")]
    bench_transform(rest)
    (own,) = person
    own["name"], own["uid"] = f"{n['person']} 1", "demo-person-1"
    texture = own["faces"][0]["texture"]
    figures = [figure(f"{n['person']} {i + 1}", f"demo-person-{i + 1}", texture["embed"], texture["sw"], texture["sh"], x, y, 0.0)
               for i, (x, y) in enumerate(PEOPLE) if i > 0]
    bank = {"name": n["bench"], "uid": "demo-bench", "component": False, "edges": [], "faces": [], "children": rest}
    scene["groups"] = [g.json() for g in theatre(n)] + [bank] + person + figures
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as z:
        info = zipfile.ZipInfo("document.json", (1980, 1, 1, 0, 0, 0))
        info.compress_type = zipfile.ZIP_DEFLATED
        z.writestr(info, json.dumps(doc, separators=(",", ":"), ensure_ascii=False))
        for k, data in sorted(textures.items()):
            info = zipfile.ZipInfo(k, (1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_STORED
            z.writestr(info, data)
    (OUT / lang / f"{NAME}.igz").write_bytes(buf.getvalue())


# ── Fixture placement (BlinderKitten's conventions, see bk-olga/src/placement.rs) ─

def beam_dir(rot):
    e = [-math.radians(d) for d in rot]
    sx, cx, sy, cy, sz, cz = math.sin(e[0]), math.cos(e[0]), math.sin(e[1]), math.cos(e[1]), math.sin(e[2]), math.cos(e[2])
    m = [cy * cz + sx * sy * sz, cx * sz, cy * sx * sz - cz * sy, 0, cz * sx * sy - cy * sz, cx * cz, cy * cz * sx + sy * sz, 0, cx * sy, -sx, cx * cy, 0]
    local = [[m[4 * j + i] for j in range(3)] for i in range(3)]
    return [-local[2][r] for r in range(3)]


def aim(p, t):
    d = [t[i] - p[i] for i in range(3)]
    n = math.sqrt(sum(c * c for c in d))
    d = [c / n for c in d]
    best = max(((sum(a * b for a, b in zip(beam_dir([rx, 0, rz]), d)), rx, rz) for rx in range(-180, 180, 2) for rz in range(-180, 180, 2)))
    return [float(best[1]), 0.0, float(best[2])]


def placements():
    pl = {}
    for i, x in enumerate((-4.8, -1.6, 1.6, 4.8)):
        pl[11 + i] = ([x, 1.8, 6.65], [0.0, 0.0, 0.0])
    for i, x in enumerate((-5.6, -1.9, 1.9, 5.6)):
        p = [x, 5.2, 0.12]
        pl[21 + i] = (p, aim(p, [x, 6.9, 3.0]))
    for s, fid in ((-1, 31), (1, 32)):
        p = [s * 6.3, -1.6, 6.6]
        pl[fid] = (p, aim(p, [s * -1.8, -1.2, 0.0]))
    for fid, x, t in ((41, -3.5, [-4.0, 4.4, 0.8]), (42, 3.5, [4.2, 4.6, 1.0])):
        p = [x, -1.6, 6.65]
        pl[fid] = (p, aim(p, t))
    pl[51] = ([LANTERN[0], LANTERN[1], 3.03], [0.0, 0.0, 0.0])
    # Front light: crossing onto the stage from the bridge.
    for fid, (x, target) in FRONT.items():
        p = [x, FRONT_Y, FRONT_Z]
        pl[fid] = (p, aim(p, [target, -0.6, 1.2]))
    return pl


# ── BlinderKitten show ────────────────────────────────────────────────────────

def params(item):
    return {p["controlAddress"]: p for p in item.get("parameters", [])}


def set_param(item, address, value):
    ps = item.setdefault("parameters", [])
    for p in ps:
        if p["controlAddress"] == address:
            p["value"] = value
            p.pop("enabled", None)
            return
    ps.append({"value": value, "controlAddress": address})


RED, GREEN, BLUE, WHITE = (f"/color/channelTypes/{c}" for c in ("red", "green", "blue", "white"))
INT, PAN, TILT, ZOOM = "/dimmer/channelTypes/intensity", "/position/channelTypes/pan", "/position/channelTypes/tilt", "/focus/channelTypes/zoom"
STROBE = "/beam/channelTypes/strobeFrequency"


def rgbw(r, g, b, w=0.0):
    return [(RED, r), (GREEN, g), (BLUE, b), (WHITE, w)]


def head(fid, target):
    """Pan and tilt (BK's 0..1) that point a hanging moving head at `target`, by its
    ±270° / ±102° ranges; at pan = tilt = 0 it points down (bk-olga/src/placement.rs)."""
    p = placements()[fid][0]
    d = [t - q for t, q in zip(target, p)]
    n = math.sqrt(sum(c * c for c in d))
    tilt = math.degrees(math.acos(-d[2] / n))
    pan = math.degrees(math.atan2(d[0], d[1]))
    return [(PAN, (pan + 270) / 540), (TILT, (tilt + 102) / 204)]


TABLE_CENTRE = (0.0, 0.6, 0.8)    # where the table stands in the party scene

# Cues of the main cuelist: (id, fade, [(group or fixture id, kind, values)]); names in NAMES.
# BlinderKitten tracks: what a cue does not set keeps its value from the cue before.
CUES = [
    (1, 3.0, [
        (2, "Group", rgbw(1.0, 0.32, 0.0)),
        (3, "Group", [(INT, 0.45)] + rgbw(1.0, 0.7, 0.4, 1.0)),
        (5, "Group", [(INT, 0.8)]),
        (6, "Group", [(INT, 0.35)] + rgbw(1.0, 0.75, 0.5, 1.0)),
    ]),
    (2, 6.0, [
        (2, "Group", rgbw(0.0, 0.1, 1.0)),
        (3, "Group", [(INT, 0.3)] + rgbw(0.4, 0.5, 1.0, 0.0)),
        (5, "Group", [(INT, 1.0)]),
        (6, "Group", [(INT, 0.15)] + rgbw(0.5, 0.6, 1.0, 0.6)),
    ]),
    (3, 4.0, [
        (1, "Group", [(INT, 1.0), (ZOOM, 0.78), (STROBE, 0.0), (PAN, 0.667)] + rgbw(0.12, 0.0, 1.0)),
        (11, "Fixture", [(TILT, 0.659)]), (12, "Fixture", [(TILT, 0.588)]),
        (13, "Fixture", [(TILT, 0.416)]), (14, "Fixture", [(TILT, 0.345)]),
        (2, "Group", rgbw(1.0, 0.0, 0.25)),
        (3, "Group", [(INT, 1.0)] + rgbw(1.0, 0.55, 0.16, 0.24)),
        (4, "Group", [(INT, 1.0)] + rgbw(0.0, 0.08, 1.0)),
        (5, "Group", [(INT, 1.0)]),
        (6, "Group", [(INT, 0.45)] + rgbw(1.0, 0.7, 0.45, 1.0)),
    ]),
    (4, 5.0, [
        (11, "Fixture", [(INT, 0.0)]), (14, "Fixture", [(INT, 0.0)]),
        (12, "Fixture", [(INT, 0.9), (ZOOM, 0.92)] + head(12, (0.3, 0.9, 0.5)) + rgbw(0.6, 0.7, 1.0, 1.0)),
        (13, "Fixture", [(INT, 0.6), (ZOOM, 0.92)] + head(13, (-0.65, -0.6, 1.0)) + rgbw(0.6, 0.7, 1.0, 1.0)),
        (2, "Group", rgbw(0.0, 0.03, 0.4)),
        (3, "Group", [(INT, 0.0)]),
        (4, "Group", [(INT, 0.3)]),
        (6, "Group", [(INT, 0.0)]),
    ]),
    (5, 5.0, [
        (1, "Group", [(INT, 0.7), (ZOOM, 0.55), (PAN, 0.5), (TILT, 0.5)] + rgbw(1.0, 0.8, 0.5, 1.0)),
        (2, "Group", rgbw(0.25, 0.65, 1.0, 0.3)),
        (3, "Group", [(INT, 1.0)] + rgbw(1.0, 0.85, 0.6, 1.0)),
        (4, "Group", [(INT, 0.8)] + rgbw(1.0, 0.45, 0.0)),
        (5, "Group", [(INT, 0.0)]),
        (6, "Group", [(INT, 0.8)] + rgbw(1.0, 0.85, 0.7, 1.0)),
    ]),
    (6, 3.0, [
        (1, "Group", [(INT, 1.0), (ZOOM, 0.6)]),
        (11, "Fixture", head(11, TABLE_CENTRE) + rgbw(1.0, 0.0, 0.0)),
        (12, "Fixture", head(12, TABLE_CENTRE) + rgbw(1.0, 0.8, 0.0)),
        (13, "Fixture", head(13, TABLE_CENTRE) + rgbw(1.0, 0.0, 1.0)),
        (14, "Fixture", head(14, TABLE_CENTRE) + rgbw(0.0, 0.9, 1.0)),
        (2, "Group", rgbw(1.0, 0.0, 0.6)),
        (3, "Group", [(INT, 1.0)] + rgbw(1.0, 0.75, 0.45, 1.0)),
        (4, "Group", [(INT, 1.0)] + rgbw(1.0, 0.5, 0.0)),
        (5, "Group", [(INT, 1.0)]),
        (6, "Group", [(INT, 0.6)] + rgbw(1.0, 0.7, 0.5, 1.0)),
    ]),
    (7, 4.0, [
        (1, "Group", [(INT, 1.0), (ZOOM, 0.3), (PAN, 0.5), (TILT, 0.5)] + rgbw(1.0, 0.9, 0.75, 1.0)),
        (2, "Group", rgbw(1.0, 0.55, 0.1, 0.5)),
        (3, "Group", [(INT, 1.0)] + rgbw(1.0, 0.85, 0.6, 1.0)),
        (4, "Group", [(INT, 1.0)] + rgbw(1.0, 0.6, 0.2, 1.0)),
        (5, "Group", [(INT, 1.0)]),
        (6, "Group", [(INT, 1.0)] + rgbw(1.0, 0.85, 0.7, 1.0)),
    ]),
    (8, 3.0, [
        (1, "Group", [(INT, 0.0)]), (2, "Group", rgbw(0.0, 0.0, 0.0, 0.0)),
        (3, "Group", [(INT, 0.0)]), (4, "Group", [(INT, 0.0)]), (5, "Group", [(INT, 0.0)]), (6, "Group", [(INT, 0.0)]),
    ]),
]

def write_olga(lang):
    n = NAMES[lang]
    show = json.loads((ROOT / "crates/bk-olga/tests/show.olga").read_text())
    # Only what the example needs: no presets, effects or programmer state from the test show.
    for key in ("preset", "effect", "programmer"):
        show[key].pop("items", None)
    # The show's own interfaces send nothing into the network (their output is off,
    # a typical node address stays as a placeholder); BlinderKitten still passes
    # everything through to the Tapetum mirrors added below.
    for itf in show["interfaces"]["items"]:
        out = itf.get("containers", {}).get("artNet", {}).get("containers", {}).get("output")
        if out:
            set_param(out, "/remoteHost", "2.0.0.10")
            set_param(out, "/enabled", False)
    names = n["fixtures"]
    pl = placements()
    # Front light: four more of the side spots' type, patched after them.
    side = next(fx for fx in show["fixture"]["items"] if params(fx)["/id"]["value"] == 31)
    for fid, address in ((61, 440), (62, 450), (63, 460), (64, 470)):
        fx = copy.deepcopy(side)
        set_param(fx, "/id", fid)
        set_param(fx["containers"]["patch"]["items"][0], "/address", address)
        show["fixture"]["items"].append(fx)
    group6 = copy.deepcopy(next(g for g in show["group"]["items"] if g["niceName"].startswith("3 - ")))
    group6["niceName"] = "6 - Front"
    set_param(group6["containers"]["selections"]["items"][0], "/id", 61)
    set_param(group6["containers"]["selections"]["items"][0], "/lastID", 64)
    show["group"]["items"].append(group6)
    for fx in show["fixture"]["items"]:
        fid = params(fx)["/id"]["value"]
        if fid in names:
            set_param(fx, "/name", names[fid])
            fx["niceName"] = f"{fid} - {names[fid]}"
        pos, rot = pl[fid]
        set_param(fx, "/position", [round(c, 3) for c in pos])
        set_param(fx, "/rotation", rot)
    for grp in show["group"]["items"]:
        gid = int(grp["niceName"].split(" - ")[0])
        new = n["groups"].get(gid)
        if new:
            set_param(grp, "/name", new)
            grp["niceName"] = f"{gid} - {new}"

    cuelist = show["cuelist"]["items"][0]
    old = cuelist["containers"]["cues"]["items"]
    template = copy.deepcopy(next(c for c in old if c["niceName"].startswith("Umabu")))
    cmd_t = template["containers"]["commands"]["items"][0]
    sel_t = cmd_t["containers"]["selections"]["items"][0]
    val_t = cmd_t["containers"]["values"]["items"][0]
    cues = []
    for (cid, fade, commands), name, chapter in zip(CUES, n["cues"], n["chapters"]):
        cue = copy.deepcopy(template)
        cue["parameters"] = [{"value": float(cid), "controlAddress": "/id"}]
        if chapter:
            cue["parameters"].append({"value": chapter, "controlAddress": "/chapterName"})
        cue["niceName"] = name
        cue["containers"]["timing"] = {"parameters": [{"value": fade, "controlAddress": "/htpInFade"}, {"value": fade, "controlAddress": "/ltpInFade"}]} if fade else {}
        items = []
        for target, kind, values in commands:
            cmd = copy.deepcopy(cmd_t)
            cmd["niceName"] = f"{kind} {target}"
            sel = copy.deepcopy(sel_t)
            sel["parameters"] = [{"value": kind, "controlAddress": "/targetType"}, {"value": target, "hexMode": False, "controlAddress": "/id"}, {"value": "", "controlAddress": "/channelType"}]
            cmd["containers"]["selections"]["items"] = [sel]
            vals = []
            for i, (ctype, v) in enumerate(values):
                val = copy.deepcopy(val_t)
                val["niceName"] = "Value" if i == 0 else f"Value {i + 1}"
                val["parameters"] = [{"value": ctype, "controlAddress": "/channelType"}, {"value": float(v), "controlAddress": "/value"}]
                vals.append(val)
            cmd["containers"]["values"]["items"] = vals
            items.append(cmd)
        cue["containers"]["commands"]["items"] = items
        cues.append(cue)
    cuelist["containers"]["cues"]["items"] = cues
    (OUT / lang / f"{NAME}.olga").write_text(json.dumps(show, indent=1, ensure_ascii=False))


# ── Tapetum project ───────────────────────────────────────────────────────────

# Groups of the theatre, not of the set.
THEATRE = ("demo-house", "demo-seats", "demo-stage", "demo-cyc", "demo-portal", "demo-curtain", "demo-masking", "demo-truss", "demo-bridge")


def scenes(names):
    obj = lambda uid: f"1/uid:{uid}"
    state = lambda dx=0.0, dy=0.0, visible=True: {"offset": [dx, dy, 0.0], "rotation": [0.0, 0.0, 0.0], "visible": visible}
    morning = {  # the crates are gone, table and chairs move up stage left
        obj("demo-crates"): state(visible=False),
        obj("demo-table"): state(-4.6, 3.2), obj("demo-chairs"): state(-4.6, 3.2),
    }
    # The lantern stays: its light is a fixture, and fixtures don't move with scenes.
    party = {  # the table centre stage, two people side by side in front of it, all else off to the side stages
        obj("demo-person-1"): state(0.25, 0.2), obj("demo-person-2"): state(-2.0, 0.1),
        obj("demo-person-3"): state(-7.5), obj("demo-person-4"): state(5.7),
        obj("demo-table"): state(-3.1, 2.0), obj("demo-chairs"): state(-3.1, 2.0),
        obj("demo-bench"): state(9.6), obj("demo-columns"): state(6.5),
        obj("demo-podest"): state(-6.2), obj("demo-crates"): state(-6.3),
    }
    # The finale: everything back where it stands in the files.
    return [{"id": i + 1, "name": name, "cue_id": cue, "transition": t, "objects": objects}
            for i, (name, cue, t, objects) in enumerate(zip(names, (5.0, 6.0, 7.0), (3.0, 4.0, 4.0), (morning, party, {})))]


def write_project(lang):
    project = {
        "render": {"ambient": 0.25, "exposure_ev": 5.6, "bloom": True,
                   "haze": {"enabled": True, "density": 0.16, "anisotropy": 0.6, "color": [1, 1, 1], "extent": [30, 12, 30]}},
        # The theatre itself is locked (it doesn't move from scene to scene); the set isn't.
        "objects": [{"id": 1, "name": f"{NAME}.igz", "model": f"{NAME}.igz", "visible": True,
                     "parts": {f"uid:{uid}": {"locked": True} for uid in THEATRE}}],
        "fixture_types": {
            "movingHeadWash": {"fixed_gobo": {"Builtin": "Stern"}},
            "theatreSpotRGBW": {"beam_angle": 26.0},
            "waterEffect": {"fixed_gobo": {"Builtin": "Wasser"}, "beam_angle": 36.0},
            "barRGBW": {"beam_angle": 75.0},
            "channel": {"beam_angle": 360.0, "lumens": 1800.0, "white_kelvin": 2700.0, "body_scale": 0.3},
        },
        "scenes": scenes(NAMES[lang]["scenes"]),
    }
    (OUT / lang / f"{NAME}.tapetum").write_text(json.dumps(project, indent=1, ensure_ascii=False) + "\n")


def add_mirrors(lang):
    """The Art-Net mirrors with pass-through, exactly as "Add the Tapetum interfaces" makes them."""
    show = OUT / lang / f"{NAME}.olga"
    subprocess.run(["cargo", "run", "-q", "--release", "-p", "tapetum", "--", "mirror", str(show), "--offline"], cwd=ROOT, check=True)
    show.with_name(show.name + ".bak").unlink(missing_ok=True)


if __name__ == "__main__":
    for lang in NAMES:
        (OUT / lang).mkdir(parents=True, exist_ok=True)
        write_igz(lang)
        write_olga(lang)
        write_project(lang)
        add_mirrors(lang)
    for f in sorted(OUT.rglob("*.*")):
        print(f"{f.relative_to(ROOT)}  {f.stat().st_size // 1024} KB")
