#!/usr/bin/env python3
"""Convert the front artwork SVG into a KiCad front board.

Mirrors how front/owadzium.kicad_pcb is built:
  - red stroke outline (Warstwa_3_kopia)  -> Edge.Cuts
  - gold fill          (Warstwa_2_kopia)  -> F.Mask openings (ENIG copper shows through)
  - black fill         (Warstwa_4_kopia_2)-> F.SilkS (white silkscreen)
  - one F.Cu zone with no net under the whole board

Board setup (layers, stackup, mask colour, plot params) is copied from a
template board, and the new outline is centred on the template's outline.

Requirements: Python 3.8+ and shapely (pip install shapely).
Zone filling uses KiCad's pcbnew module when available. On Windows, run the
script from the "KiCad Command Prompt", whose python has pcbnew
(install shapely there with: python -m pip install shapely).
Without pcbnew the zone is left unfilled; press B in pcbnew to fill it.

Usage (defaults are relative to this script's folder):
  python svg2front.py
  python svg2front.py --svg art.svg --template ../front/owadzium.kicad_pcb --out owadzium.kicad_pcb
"""
import argparse
import math
import re
import uuid
import xml.etree.ElementTree as ET
from pathlib import Path

from shapely import affinity
from shapely.geometry import MultiPolygon, Polygon, box
from shapely.ops import unary_union
from shapely.validation import make_valid

HERE = Path(__file__).resolve().parent
NS = "{http://www.w3.org/2000/svg}"
PT = 25.4 / 72  # Illustrator SVG user unit is 1 pt
SLIT = 0.001    # mm; hairline cut that opens holes, far below mask resolution

ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
ap.add_argument("--svg", type=Path, default=HERE / "obgrysnpwy_all.svg")
ap.add_argument("--template", type=Path, default=HERE.parent / "front" / "owadzium.kicad_pcb",
                help="board to copy setup from and centre the outline on")
ap.add_argument("--out", type=Path, default=HERE / "owadzium.kicad_pcb")
ap.add_argument("--outline-layer", default="Warstwa_3_kopia")
ap.add_argument("--gold-layer", default="Warstwa_2_kopia")
ap.add_argument("--black-layer", default="Warstwa_4_kopia_2")
ap.add_argument("--black-to", default="F.SilkS", help="KiCad layer for the black artwork")
ap.add_argument("--edge-clearance", type=float, default=0.5, help="zone clearance, mm")
ap.add_argument("--tolerance", type=float, default=0.01, help="curve flattening tolerance, mm")
args = ap.parse_args()
TOL = args.tolerance / PT

# ---------- SVG path parsing ----------
TOK = re.compile(r"[MmLlHhVvCcSsQqTtAaZz]|[-+]?(?:\d*\.\d+|\d+\.?)(?:[eE][-+]?\d+)?")


def bez(p0, p1, p2, p3):
    dd = (abs(p0[0] - 2 * p1[0] + p2[0]) + abs(p1[0] - 2 * p2[0] + p3[0]) +
          abs(p0[1] - 2 * p1[1] + p2[1]) + abs(p1[1] - 2 * p2[1] + p3[1]))
    n = max(2, math.ceil(math.sqrt(dd * 0.75 / TOL)))
    out = []
    for i in range(1, n + 1):
        t = i / n
        u = 1 - t
        out.append((u**3 * p0[0] + 3 * u * u * t * p1[0] + 3 * u * t * t * p2[0] + t**3 * p3[0],
                    u**3 * p0[1] + 3 * u * u * t * p1[1] + 3 * u * t * t * p2[1] + t**3 * p3[1]))
    return out


def arc(p0, rx, ry, phi, fa, fs, p1):
    """SVG elliptical arc, endpoint -> centre parameterisation."""
    if rx == 0 or ry == 0:
        return [p1]
    phi = math.radians(phi)
    c, s = math.cos(phi), math.sin(phi)
    dx, dy = (p0[0] - p1[0]) / 2, (p0[1] - p1[1]) / 2
    x1, y1 = c * dx + s * dy, -s * dx + c * dy
    rx, ry = abs(rx), abs(ry)
    lam = x1 * x1 / (rx * rx) + y1 * y1 / (ry * ry)
    if lam > 1:
        rx *= math.sqrt(lam)
        ry *= math.sqrt(lam)
    num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1
    k = math.sqrt(max(0, num / (rx * rx * y1 * y1 + ry * ry * x1 * x1)))
    if fa == fs:
        k = -k
    cx1, cy1 = k * rx * y1 / ry, -k * ry * x1 / rx
    cx = c * cx1 - s * cy1 + (p0[0] + p1[0]) / 2
    cy = s * cx1 + c * cy1 + (p0[1] + p1[1]) / 2
    a0 = math.atan2((y1 - cy1) / ry, (x1 - cx1) / rx)
    da = math.atan2((-y1 - cy1) / ry, (-x1 - cx1) / rx) - a0
    if fs and da < 0:
        da += 2 * math.pi
    if not fs and da > 0:
        da -= 2 * math.pi
    r = max(rx, ry)
    n = max(4, int(abs(da) * r / (2 * math.sqrt(2 * r * TOL)) + 1))
    pts = []
    for i in range(1, n + 1):
        a = a0 + da * i / n
        pts.append((cx + rx * math.cos(a) * c - ry * math.sin(a) * s,
                    cy + rx * math.cos(a) * s + ry * math.sin(a) * c))
    return pts


def parse_path(d):
    """Return a list of flattened subpaths (lists of (x, y))."""
    toks = TOK.findall(d)
    i = 0
    cmd = None
    subs, cur = [], []
    x = y = sx = sy = 0.0
    lc, lcmd = None, ""

    def num():
        nonlocal i
        i += 1
        return float(toks[i - 1])

    def flag():
        # arc flags may be packed, e.g. "01"
        nonlocal i
        t = toks[i]
        if len(t) > 1 and t[0] in "01" and not t.startswith(("0.", "1.")):
            toks[i] = t[1:]
            return int(t[0])
        i += 1
        return int(float(t))

    while i < len(toks):
        if toks[i].isalpha():
            cmd = toks[i]
            i += 1
            if cmd in "Zz":
                if cur:
                    subs.append(cur)
                cur = []
                x, y = sx, sy
                lcmd = "Z"
                continue
        rel = cmd.islower()
        C = cmd.upper()
        ox, oy = (x, y) if rel else (0.0, 0.0)
        if C == "M":
            if cur:
                subs.append(cur)
            x, y = num() + ox, num() + oy
            sx, sy = x, y
            cur = [(x, y)]
            cmd = "l" if rel else "L"
        elif C == "L":
            x, y = num() + ox, num() + oy
            cur.append((x, y))
        elif C == "H":
            x = num() + (x if rel else 0)
            cur.append((x, y))
        elif C == "V":
            y = num() + (y if rel else 0)
            cur.append((x, y))
        elif C == "C":
            p1 = (num() + ox, num() + oy)
            p2 = (num() + ox, num() + oy)
            p3 = (num() + ox, num() + oy)
            cur += bez((x, y), p1, p2, p3)
            lc = p2
            x, y = p3
        elif C == "S":
            p1 = (2 * x - lc[0], 2 * y - lc[1]) if lcmd in ("C", "S") else (x, y)
            p2 = (num() + ox, num() + oy)
            p3 = (num() + ox, num() + oy)
            cur += bez((x, y), p1, p2, p3)
            lc = p2
            x, y = p3
        elif C == "Q":
            q = (num() + ox, num() + oy)
            p3 = (num() + ox, num() + oy)
            p1 = (x + 2 / 3 * (q[0] - x), y + 2 / 3 * (q[1] - y))
            p2 = (p3[0] + 2 / 3 * (q[0] - p3[0]), p3[1] + 2 / 3 * (q[1] - p3[1]))
            cur += bez((x, y), p1, p2, p3)
            lc = q
            x, y = p3
        elif C == "A":
            rx, ry, ph = num(), num(), num()
            fa, fs = flag(), flag()
            p3 = (num() + ox, num() + oy)
            cur += arc((x, y), rx, ry, ph, fa, fs, p3)
            x, y = p3
        else:
            raise ValueError(f"unsupported path command {cmd!r}")
        lcmd = C
    if cur:
        subs.append(cur)
    return subs


def fill_geom(subs):
    # Compound paths as even-odd; Illustrator holes have opposite winding, so same result
    g = Polygon()
    for s in subs:
        if len(s) >= 3:
            g = g.symmetric_difference(make_valid(Polygon(s)))
    return g


def apply_transform(geom, t):
    ops = re.findall(r"(\w+)\(([^)]*)\)", t or "")
    for name, a in reversed(ops):
        a = [float(v) for v in re.split(r"[ ,]+", a.strip())]
        if name == "translate":
            geom = affinity.translate(geom, a[0], a[1] if len(a) > 1 else 0)
        elif name == "rotate":
            geom = affinity.rotate(geom, a[0], origin=(a[1], a[2]) if len(a) == 3 else (0, 0))
        elif name == "scale":
            geom = affinity.scale(geom, a[0], a[1] if len(a) > 1 else a[0], origin=(0, 0))
        elif name == "matrix":
            geom = affinity.affine_transform(geom, [a[0], a[2], a[1], a[3], a[4], a[5]])
        else:
            raise ValueError(f"unsupported transform {name}")
    return geom


def collect_fill(g):
    out = []
    for el in g.iter():
        tag = el.tag.replace(NS, "")
        if tag == "path":
            geom = fill_geom(parse_path(el.get("d")))
        elif tag in ("circle", "ellipse"):
            cx, cy = float(el.get("cx", 0)), float(el.get("cy", 0))
            rx = float(el.get("r") or el.get("rx"))
            ry = float(el.get("r") or el.get("ry"))
            geom = Polygon([(cx + rx * math.cos(2 * math.pi * k / 32), cy + ry * math.sin(2 * math.pi * k / 32))
                            for k in range(32)])
        elif tag == "rect":
            x, y = float(el.get("x", 0)), float(el.get("y", 0))
            geom = box(x, y, x + float(el.get("width")), y + float(el.get("height")))
        elif tag == "polygon":
            nums = [float(v) for v in re.split(r"[ ,]+", el.get("points").strip())]
            geom = make_valid(Polygon(list(zip(nums[::2], nums[1::2]))))
        else:
            continue
        out.append(apply_transform(geom, el.get("transform")))
    return unary_union(out)


def polys(g):
    g = make_valid(g)
    if isinstance(g, Polygon):
        return [g]
    res = []
    for p in getattr(g, "geoms", []):
        if isinstance(p, Polygon):
            res.append(p)
        elif isinstance(p, MultiPolygon):
            res += list(p.geoms)
    return res


def no_holes(g):
    """gr_poly can't have holes: cut a hairline slit upward from each hole."""
    res = []
    for p in polys(g):
        if not p.interiors:
            res.append(p)
            continue
        cuts = []
        for ring in p.interiors:
            x, y = min(ring.coords, key=lambda c: c[1])  # topmost point
            cuts.append(box(x - SLIT / 2, p.bounds[1] - 1, x + SLIT / 2, y + 1e-6))
        res += no_holes(p.difference(unary_union(cuts)))
    return [r for r in res if r.area > 1e-5]


# ---------- read inputs ----------
root = ET.parse(args.svg).getroot()
layers = {g.get("id"): g for g in root.iter(f"{NS}g") if g.get("id")}
for name in (args.outline_layer, args.gold_layer, args.black_layer):
    if name not in layers:
        raise SystemExit(f"SVG layer {name!r} not found; available: {', '.join(layers)}")

outline_subs = [s for p in layers[args.outline_layer].iter(f"{NS}path") for s in parse_path(p.get("d"))]
if len(outline_subs) != 1:
    raise SystemExit(f"expected one outline path, found {len(outline_subs)}")
outline = Polygon(outline_subs[0])
gold = collect_fill(layers[args.gold_layer])
black = collect_fill(layers[args.black_layer])

template = args.template.read_text(encoding="utf-8")
if "\t(gr_poly" not in template:
    raise SystemExit("template has no gr_poly; can't find where the board setup ends")
header = template[:template.find("\t(gr_poly")]

# centre on the template's Edge.Cuts outline
tpl_pts = []
for blk in template.split("\t(gr_poly")[1:]:
    if '(layer "Edge.Cuts")' in blk.split("\n\t)")[0]:
        tpl_pts += [(float(a), float(b)) for a, b in
                    re.findall(r"\(xy ([-\d.]+) ([-\d.]+)\)", blk.split("(stroke")[0])]
if tpl_pts:
    xs, ys = zip(*tpl_pts)
    tcx, tcy = round((min(xs) + max(xs)) / 2, 1), round((min(ys) + max(ys)) / 2, 1)
else:
    tcx, tcy = 100.0, 100.0

minx, miny, maxx, maxy = outline.bounds
cx, cy = (minx + maxx) / 2, (miny + maxy) / 2


def to_board(g):
    g = affinity.translate(g, -cx, -cy)
    g = affinity.scale(g, PT, PT, origin=(0, 0))
    return affinity.translate(g, tcx, tcy)


outline, gold, black = to_board(outline), to_board(gold), to_board(black)

# ---------- write board ----------
uuids = []


def gr_poly(p, layer, fill, width):
    u = str(uuid.uuid4())
    uuids.append(u)
    coords = list(p.simplify(0.002).exterior.coords)[:-1]
    rows = "\n".join("\t\t\t" + " ".join(f"(xy {x:.6f} {y:.6f})" for x, y in coords[i:i + 4])
                     for i in range(0, len(coords), 4))
    return (f"\t(gr_poly\n\t\t(pts\n{rows}\n\t\t)\n"
            f"\t\t(stroke\n\t\t\t(width {width})\n\t\t\t(type solid)\n\t\t)\n"
            f"\t\t(fill {fill})\n\t\t(layer \"{layer}\")\n\t\t(uuid \"{u}\")\n\t)\n")


gold_parts = no_holes(gold)
black_parts = no_holes(black)
items = [gr_poly(p, "F.Mask", "yes", 0) for p in gold_parts]
items += [gr_poly(p, args.black_to, "yes", 0) for p in black_parts]
items.append(gr_poly(outline, "Edge.Cuts", "no", 0.05))

bx0, by0, bx1, by1 = outline.bounds
c = args.edge_clearance
zone = f"""\t(zone
\t\t(layer "F.Cu")
\t\t(uuid "{uuid.uuid4()}")
\t\t(hatch edge 0.5)
\t\t(connect_pads
\t\t\t(clearance {c})
\t\t)
\t\t(min_thickness 0.25)
\t\t(fill yes
\t\t\t(thermal_gap 0.5)
\t\t\t(thermal_bridge_width 0.5)
\t\t\t(island_removal_mode 1)
\t\t)
\t\t(polygon
\t\t\t(pts
\t\t\t\t(xy {bx0 - 2:.1f} {by0 - 2:.1f}) (xy {bx1 + 2:.1f} {by0 - 2:.1f}) (xy {bx1 + 2:.1f} {by1 + 2:.1f}) (xy {bx0 - 2:.1f} {by1 + 2:.1f})
\t\t\t)
\t\t)
\t)
"""
members = "\n".join("\t\t\t" + " ".join(f'"{u}"' for u in uuids[i:i + 2]) for i in range(0, len(uuids), 2))
group = f'\t(group ""\n\t\t(uuid "{uuid.uuid4()}")\n\t\t(members\n{members}\n\t\t)\n\t)\n'

# newline="\n": keep KiCad's LF line endings on Windows too
with open(args.out, "w", encoding="utf-8", newline="\n") as f:
    f.write(header + "".join(items) + zone + group + "\t(embedded_fonts no)\n)\n")

print(f"board size: {bx1 - bx0:.3f} x {by1 - by0:.3f} mm")
print(f"{len(gold_parts)} mask openings, {len(black_parts)} shapes on {args.black_to}")

try:
    import pcbnew
except ImportError:
    print("pcbnew not available: zone left unfilled, press B in pcbnew to fill it")
else:
    board = pcbnew.LoadBoard(str(args.out))
    pcbnew.ZONE_FILLER(board).Fill(board.Zones())
    board.Save(str(args.out))
    print("zone filled")
print(f"wrote {args.out}")
