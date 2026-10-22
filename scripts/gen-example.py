#!/usr/bin/env python3
"""Builds the example that ships inside Tapetum (crates/tapetum/assets/example/):

  Tapetum-Demo.igz      a small theatre as an IngeTrazo document (format 2, a ZIP
                        with document.json + textures/), with the pergola bench of
                        crates/tapetum/tests/data (CC BY 4.0, Marco Sumari Tellez)
  Tapetum-Demo.olga     a BlinderKitten show: the fixtures of the test show, hung in
                        that theatre, with a few cues
  Tapetum-Demo.tapetum  Tapetum's project: the .igz, fixture settings, a scene

Everything is modelled here; nothing is downloaded. Run after changing it:

    python3 scripts/gen-example.py
"""
import copy
import io
import json
import math
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "crates/tapetum/assets/example"
NAME = "Tapetum-Demo"

# ── Geometry (IngeTrazo: metres, Z up, the audience looks along +Y) ──────────
# Faces are counterclockwise seen from outside, colours (r, g, b, 1) in 0..1.

STAGE_FRONT, STAGE_BACK = -4.4, 7.2


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


def theatre():
    G = []
    def group(name, uid, rgb):
        g = Group(name, uid, (*rgb, 1.0))
        G.append(g)
        return g

    house = group("Zuschauerraum Boden", "demo-house", (0.05, 0.04, 0.04))
    house.box(-16, 16, -18, STAGE_FRONT, -1.02, -1.0)
    seats = group("Zuschauerreihen", "demo-seats", (0.28, 0.03, 0.04))
    for r in range(7):
        y, z = -7.2 - r * 1.05, -1.0 + r * 0.16
        if r:
            house.box(-16, 16, y - 0.6, y + 0.45, z - 0.02, z)
        for k in range(-12, 13):
            if k:
                x = k * 0.56
                seats.box(x - 0.24, x + 0.24, y, y + 0.09, z + 0.42, z + 1.0)
                seats.box(x - 0.24, x + 0.24, y - 0.5, y, z + 0.38, z + 0.5)
    group("Bühne", "demo-stage", (0.30, 0.28, 0.27)).box(-9, 9, STAGE_FRONT, STAGE_BACK, -1.0, 0.0)
    group("Rundhorizont", "demo-cyc", (0.82, 0.82, 0.85)).face((-9, 6.9, 0), (9, 6.9, 0), (9, 6.9, 9), (-9, 6.9, 9))
    portal = group("Portal", "demo-portal", (0.09, 0.02, 0.025))
    portal.box(-11, -6.4, -5.0, -4.6, -1, 9.5)
    portal.box(6.4, 11, -5.0, -4.6, -1, 9.5)
    portal.box(-6.4, 6.4, -5.0, -4.6, 6.6, 9.5)
    curtain = group("Vorhang", "demo-curtain", (0.55, 0.035, 0.05))
    drape(curtain, -6.4, -4.3, -5.35, -4.95, -4.45, 0, 6.6, 2.2)
    drape(curtain, 6.4, 4.3, 5.35, 4.95, -4.45, 0, 6.6, 2.2)
    valance(curtain, -6.4, 6.4, -4.4, 5.9, 6.6)
    masking = group("Gassen und Soffitten", "demo-masking", (0.025, 0.025, 0.025))
    for y in (0.0, 3.4):
        for s in (-1, 1):
            masking.box(min(s * 5.3, s * 7.6), max(s * 5.3, s * 7.6), y, y + 0.05, 0, 8)
        masking.box(-7.6, 7.6, y - 0.4, y - 0.35, 7.4, 8.6)
    truss = group("Traversen", "demo-truss", (0.5, 0.5, 0.52))
    for y in (-1.6, 1.8):
        for dy in (-0.15, 0.15):
            for dz in (-0.15, 0.15):
                truss.box(-6.5, 6.5, y + dy - 0.02, y + dy + 0.02, 7.05 + dz - 0.02, 7.05 + dz + 0.02)

    podest = group("Podest", "demo-podest", (0.22, 0.2, 0.22))
    podest.box(-5.6, -2.4, 3.4, 5.8, 0, 0.8)
    for k in range(3):
        podest.box(-4.6, -3.2, 3.4 - 0.32 * (3 - k), 3.4 - 0.32 * (2 - k), 0, 0.2 * (k + 1))
    columns = group("Säulen", "demo-columns", (0.78, 0.74, 0.68))
    for cx, cy in ((3.4, 4.9), (5.0, 4.3)):
        columns.box(cx - 0.36, cx + 0.36, cy - 0.36, cy + 0.36, 0, 0.25)
        columns.cylinder(cx, cy, 0.24, 0.25, 3.9)
        columns.box(cx - 0.36, cx + 0.36, cy - 0.36, cy + 0.36, 3.9, 4.15)
    columns.box(3.0, 5.4, 4.0, 5.2, 4.15, 4.45)
    tx, ty = 3.1, -1.4
    table = group("Tisch", "demo-table", (0.42, 0.25, 0.13))
    table.box(tx - 0.55, tx + 0.55, ty - 0.38, ty + 0.38, 0.72, 0.77)
    for sx in (-0.48, 0.48):
        for sy in (-0.31, 0.31):
            table.box(tx + sx - 0.03, tx + sx + 0.03, ty + sy - 0.03, ty + sy + 0.03, 0, 0.72)
    chairs = group("Stühle", "demo-chairs", (0.42, 0.25, 0.13))
    for side in (-1, 1):
        cx = tx + side * 0.95
        chairs.box(cx - 0.22, cx + 0.22, ty - 0.22, ty + 0.22, 0.43, 0.47)
        for sx in (-0.19, 0.19):
            for sy in (-0.19, 0.19):
                chairs.box(cx + sx - 0.02, cx + sx + 0.02, ty + sy - 0.02, ty + sy + 0.02, 0, 0.43)
        bx = cx + side * 0.2
        chairs.box(bx - 0.02, bx + 0.02, ty - 0.22, ty + 0.22, 0.47, 0.95)
    lx, ly = LANTERN
    lantern = group("Laterne", "demo-lantern", (0.08, 0.08, 0.09))
    lantern.box(lx - 0.15, lx + 0.15, ly - 0.15, ly + 0.15, 0, 0.12)
    lantern.cylinder(lx, ly, 0.045, 0.12, 2.85, 10)
    lantern.box(lx - 0.17, lx + 0.17, ly - 0.17, ly + 0.17, 3.2, 3.26)
    lantern.box(lx - 0.13, lx + 0.13, ly - 0.13, ly + 0.13, 2.83, 2.87)
    for sx in (-0.13, 0.13):
        for sy in (-0.13, 0.13):
            lantern.box(lx + sx - 0.012, lx + sx + 0.012, ly + sy - 0.012, ly + sy + 0.012, 2.87, 3.2)
    crates = group("Kisten", "demo-crates", (0.5, 0.37, 0.2))
    for x, y, z, a in ((-5.0, 0.9, 0, 0.7), (-4.2, 1.1, 0, 0.6), (-4.65, 1.0, 0.7, 0.55), (-5.3, 2.0, 0, 0.5)):
        crates.box(x - a / 2, x + a / 2, y - a / 2, y + a / 2, z, z + a)
    return G


LANTERN = (-3.3, -1.9)


def write_igz():
    bench = ROOT / "crates/tapetum/tests/data/banca-pergola-yanque.igz"
    with zipfile.ZipFile(bench) as z:
        doc = json.loads(z.read("document.json"))
        textures = {n: z.read(n) for n in z.namelist() if n.startswith("textures/")}
    doc["app_version"] = "0.5.7"
    scene = doc["scene"]
    # The bench's own groups become one named group "Bank", the face-me figure stays apart.
    bench_groups = scene["groups"]
    figure = [g for g in bench_groups if g.get("billboard")]
    rest = [g for g in bench_groups if not g.get("billboard")]
    for g in figure:
        g["name"], g["uid"] = "Person", "demo-person"
    bank = {"name": "Bank", "uid": "demo-bench", "component": False, "edges": [], "faces": [], "children": rest}
    scene["groups"] = [g.json() for g in theatre()] + [bank] + figure
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as z:
        info = zipfile.ZipInfo("document.json", (1980, 1, 1, 0, 0, 0))
        info.compress_type = zipfile.ZIP_DEFLATED
        z.writestr(info, json.dumps(doc, separators=(",", ":")))
        for n, data in sorted(textures.items()):
            info = zipfile.ZipInfo(n, (1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_STORED
            z.writestr(info, data)
    (OUT / f"{NAME}.igz").write_bytes(buf.getvalue())


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


# Cues of the main cuelist: (id, name, chapter, fade, [(group or fixture id, kind, values)]).
CUES = [
    (1, "Einlass", "1 - Einlass", 0.0, [
        (2, "Group", rgbw(1.0, 0.32, 0.0)),
        (3, "Group", [(INT, 0.45)] + rgbw(1.0, 0.7, 0.4, 1.0)),
        (5, "Group", [(INT, 0.8)]),
    ]),
    (2, "Abend", "2 - Abend", 4.0, [
        (1, "Group", [(INT, 1.0), (ZOOM, 0.78), (STROBE, 0.0), (PAN, 0.667)] + rgbw(0.12, 0.0, 1.0)),
        (11, "Fixture", [(TILT, 0.659)]), (12, "Fixture", [(TILT, 0.588)]),
        (13, "Fixture", [(TILT, 0.416)]), (14, "Fixture", [(TILT, 0.345)]),
        (2, "Group", rgbw(1.0, 0.0, 0.25)),
        (3, "Group", [(INT, 1.0)] + rgbw(1.0, 0.55, 0.16, 0.24)),
        (4, "Group", [(INT, 1.0), (STROBE, 0.0)] + rgbw(0.0, 0.08, 1.0)),
        (5, "Group", [(INT, 1.0)]),
    ]),
    (3, "Morgen", "3 - Morgen", 5.0, [
        (1, "Group", [(INT, 0.7), (ZOOM, 0.55), (PAN, 0.5), (TILT, 0.5)] + rgbw(1.0, 0.8, 0.5, 1.0)),
        (2, "Group", rgbw(0.25, 0.65, 1.0, 0.3)),
        (3, "Group", [(INT, 1.0)] + rgbw(1.0, 0.85, 0.6, 1.0)),
        (4, "Group", [(INT, 0.8)] + rgbw(1.0, 0.45, 0.0)),
        (5, "Group", [(INT, 0.0)]),
    ]),
    (4, "Blackout", "", 3.0, [
        (1, "Group", [(INT, 0.0)]), (2, "Group", rgbw(0.0, 0.0, 0.0)),
        (3, "Group", [(INT, 0.0)]), (4, "Group", [(INT, 0.0)]), (5, "Group", [(INT, 0.0)]),
    ]),
]


def write_olga():
    show = json.loads((ROOT / "crates/bk-olga/tests/show.olga").read_text())
    # Only what the example needs: no presets, effects or programmer state from the test show.
    for key in ("preset", "effect", "programmer"):
        show[key].pop("items", None)
    # Art-Net to a typical node address instead of the test show's own network.
    for itf in show["interfaces"]["items"]:
        out = itf.get("containers", {}).get("artNet", {}).get("containers", {}).get("output")
        if out:
            set_param(out, "/remoteHost", "2.0.0.10")
    names = {41: "Wash L", 42: "Wash R", 51: "Laterne"}
    pl = placements()
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
        new = {4: "Wash", 5: "Laterne"}.get(gid)
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
    for cid, name, chapter, fade, commands in CUES:
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
    (OUT / f"{NAME}.olga").write_text(json.dumps(show, indent=1, ensure_ascii=False))


# ── Tapetum project ───────────────────────────────────────────────────────────

def write_project():
    obj = lambda uid: f"1/uid:{uid}"
    project = {
        "render": {"ambient": 0.25, "exposure_ev": 5.6, "bloom": True,
                   "haze": {"enabled": True, "density": 0.16, "anisotropy": 0.6, "color": [1, 1, 1], "extent": [30, 12, 30]}},
        "objects": [{"id": 1, "name": f"{NAME}.igz", "model": f"{NAME}.igz", "visible": True}],
        "fixture_types": {
            "euroliteTMH_W555": {"fixed_gobo": {"Builtin": "Stern"}},
            "varytecLEDTheaterSpot120FC": {"fixed_gobo": {"Builtin": "Wasser"}, "beam_angle": 26.0},
            "euroliteWF_40": {"beam_angle": 36.0},
            "elationPaladinBrick": {"beam_angle": 75.0},
            "channel": {"beam_angle": 360.0, "lumens": 1800.0, "white_kelvin": 2700.0, "body_scale": 0.3},
        },
        # The set change for the morning: crates gone, table and chairs centre stage.
        "scenes": [{"id": 1, "name": "Morgen", "cue_id": 3.0, "transition": 3.0, "objects": {
            obj("demo-crates"): {"offset": [0, 0, 0], "rotation": [0, 0, 0], "visible": False},
            obj("demo-table"): {"offset": [-4.6, 3.2, 0], "rotation": [0, 0, 0], "visible": True},
            obj("demo-chairs"): {"offset": [-4.6, 3.2, 0], "rotation": [0, 0, 0], "visible": True},
        }}],
    }
    (OUT / f"{NAME}.tapetum").write_text(json.dumps(project, indent=1, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    OUT.mkdir(parents=True, exist_ok=True)
    write_igz()
    write_olga()
    write_project()
    for f in sorted(OUT.iterdir()):
        print(f"{f.relative_to(ROOT)}  {f.stat().st_size // 1024} KB")
