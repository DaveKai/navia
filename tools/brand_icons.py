#!/usr/bin/env python3
"""Generates Navia's icons and brand art from assets/brand/.

  python3 -I tools/brand_icons.py          Regenerate everything.
  python3 -I tools/brand_icons.py check    Exit 1 if anything is out of date.

Needs rsvg-convert (librsvg), Pillow, numpy and fontTools.

Outputs:
- macOS: apps/navia-apple/Navia/AppIcon.icon, an Icon Composer bundle with the mark as its only layer.
  macOS draws the squircle, background, glass and tints itself.
- Linux: packaging/linux/icons/, the dark icon as SVG and hicolor PNGs. app-light.svg is for later.
- Windows: apps/navia-gpui/assets/windows/app.ico, the dark icon from 16 to 256 px. app-light.ico is for later.
- Windows installer: packaging/windows/navia-splash.gif, the mark drawing itself in and the "navia" wordmark
  fading in beside it, shown by Velopack's setup. navia-splash.png is the last frame.
- Tray: apps/navia-gpui/assets/tray/, the bare mark. com.davekai.Navia.tray-symbolic.svg follows Plasma's color scheme, and
  mark-white-N.png and mark-black-N.png are for dark and light panels.
- Sign-in lockup in GPUI: apps/navia-gpui/assets/icons/nv-brand-mark.svg and nv-brand-word.svg.
- Brand files: assets/brand/navia-wordmark-*.svg and navia-lockup-horizontal-*.svg, with the letters as paths
  so no font is needed.

The Linux and Windows icons use the brand's squircle and mark with macOS's icon background (BACKGROUND), so
every platform matches.
"""
import json, math, os, re, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BRAND = os.path.join(ROOT, "assets/brand")
MAC_ICON = os.path.join(ROOT, "apps/navia-apple/Navia/AppIcon.icon")
LINUX = os.path.join(ROOT, "packaging/linux/icons")
WINDOWS_ICO = os.path.join(ROOT, "apps/navia-gpui/assets/windows/app.ico")
TRAY = os.path.join(ROOT, "apps/navia-gpui/assets/tray")
WINDOWS_SPLASH_GIF = os.path.join(ROOT, "packaging/windows/navia-splash.gif")
WINDOWS_SPLASH_PNG = os.path.join(ROOT, "packaging/windows/navia-splash.png")
# The lockup: the mark, then "navia" in Mona Sans 750 / width 125. Sizes are fractions of the mark's ink and
# match BRAND_WORD_RATIO and BRAND_GAP_RATIO in theme.rs and Tokens.swift.
BRAND_WORD = "navia"
BRAND_WORD_FONT = "fonts/MonaSans.ttf"
BRAND_WORD_AXES = {"wght": 750, "wdth": 125}
BRAND_WORD_TRACKING = -0.015
BRAND_WORD_RATIO, BRAND_GAP_RATIO = 1.1, 0.3
BRAND_STROKE = 4.4         # the mark's stroke, in its 100-unit box
# GPUI draws the lockup from two monochrome SVGs. The wordmark is paths, since GPUI has no letter spacing or
# font variations.
GPUI_MARK = os.path.join(ROOT, "apps/navia-gpui/assets/icons/nv-brand-mark.svg")
GPUI_WORD = os.path.join(ROOT, "apps/navia-gpui/assets/icons/nv-brand-word.svg")
# The installer splash. Velopack shows the GIF while installing, and the PNG is its last frame.
SPLASH_W, SPLASH_H = 640, 320
SPLASH_MARK = 92           # the mark's ink on screen, px
SPLASH_INK = "#FFFFFF"
SPLASH_BG_GRADIENT = "d"   # id of the background gradient in navia-app-icon-dark.svg
# The mark: x = 50 + R·sin(FX·t + PHASE), y = 50 + R·sin(FY·t) in a 100-unit box, drawn as
# MARK_STEPS cubic Béziers whose handles follow the derivative.
MARK_R, MARK_FX, MARK_FY, MARK_PHASE, MARK_STEPS = 38, 3, 4, math.pi / 4, 96
# The stroke draws in, then the wordmark fades in while sliding up.
SPLASH_FPS = 15
SPLASH_DRAW_S, SPLASH_DRAW_EASE = 1.6, (0.65, 0, 0.35, 1)
SPLASH_WORD_START_S, SPLASH_WORD_S, SPLASH_WORD_EASE, SPLASH_WORD_SLIDE = 1.2, 0.6, (0, 0, 0.58, 1), 6
# Velopack plays a GIF at the average of its frame delays, so all frames share one delay and the hold
# repeats the last frame. Each frame costs the setup a full bitmap, so the hold is short.
SPLASH_HOLD_S = 1.2

LINUX_SIZES = [16, 22, 24, 32, 48, 64, 128, 256, 512]
TRAY_SIZES = [16, 22, 24, 32, 48, 64]
# The tray mark's share of the square and its line width in pixels. Small sizes get a thicker stroke
# than the brand's so the line stays visible. The symbolic SVG targets 22 px panels.
TRAY_FILL = 0.76
TRAY_LINE_PX = 1.25
TRAY_SVG_PX = 22
ICO_SIZES = [16, 20, 24, 32, 40, 48, 64, 256]
CANVAS = 1024  # Icon Composer's canvas, which the system's squircle fills.
MARK_SIZE = 72  # percent of the canvas the brandmark's 100-unit artwork covers

# macOS's own app-icon background, sampled from ictool renders at 1024 px. Light is flat white with a
# slightly darker edge. Dark is a vertical gradient with a bright rim at the top and a fainter one at
# the bottom, drawn as an inside stroke whose opacity follows the height.
BACKGROUND = {
    "light": {"top": "#FFFFFF", "bottom": "#FFFFFF",
              "rim": "#000000", "rim_stops": [(0, 0.07), (1, 0.07)]},
    "dark": {"top": "#1F1F1F", "bottom": "#0F0F0F",
             "rim": "#FFFFFF", "rim_stops": [(0, 0.30), (0.06, 0.14), (0.94, 0.14), (1, 0.22)]},
}
RIM_WIDTH = 0.9  # in the brand's 100-unit artwork, inside the squircle (about 12 px of 1024 on the Mac)


def brand(name):
    with open(os.path.join(BRAND, name)) as f:
        return f.read()


def render_png(svg_bytes, size):
    out = subprocess.run(["rsvg-convert", "-w", str(size), "-h", str(size), "-f", "png"],
                         input=svg_bytes, capture_output=True, check=True)
    return out.stdout


def mac_mark_svg(name):
    """A brandmark file scaled into the middle of the canvas. macOS draws the squircle and background."""
    src = brand(name)
    inner = re.search(r'<svg[^>]*>(.*)</svg>', src, re.S).group(1)
    # MARK_SIZE keeps the mark inside Apple's icon grid, at the same share as the container icons.
    k = CANVAS * MARK_SIZE / 100 / 100  # canvas units per mark unit
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{CANVAS}" height="{CANVAS}" viewBox="0 0 {CANVAS} {CANVAS}">'
            f'<g transform="translate({CANVAS / 2:g} {CANVAS / 2:g}) scale({k:.4f}) translate(-50 -50)">'
            f'{inner}</g></svg>\n')


def mac_icon_json():
    # Icon Composer only accepts light (no key), "dark" and "tinted". The system draws the other
    # tinted and clear appearances from "tinted".
    return {
        "groups": [{
            "layers": [{
                "name": "mark",
                "glass": True,
                # Two colored SVGs, because a layer fill also filled the inside of the lattice in the
                # flattened AppIcon.icns.
                "image-name-specializations": [
                    {"value": "mark-black.svg"},
                    {"appearance": "dark", "value": "mark-white.svg"},
                    {"appearance": "tinted", "value": "mark-white.svg"},
                ],
            }],
            "shadow": {"kind": "neutral", "opacity": 0.5},
            "specular": True,
            "translucency": {"enabled": True, "value": 0.5},
        }],
        "supported-platforms": {"squares": "shared"},
    }


# How much of a Linux or Windows icon's square the container fills. The brand file's 80% looks small
# next to Breeze and Windows icons.
DESKTOP_ICON_FILL = 1.0
# The mark's scale inside the container. 1.03 makes it span 60% of the container's width.
DESKTOP_MARK_SCALE = 1.03
# Linux and Windows imitate the Mac's glass mark with a gradient on the stroke, and its rounder
# container with a superellipse. The Mac's line is also a little thinner.
DESKTOP_SQUIRCLE_N = 5
DESKTOP_MARK_STROKE = 0.8
DESKTOP_MARK_STOPS = [(0, "#FFFFFF"), (0.45, "#E4E4E8"), (1, "#8E8E96")]


def container_svg(variant, fill=None):
    """The brand's container icon with macOS's background. `fill` crops the view so the container
    spans that share of the square."""
    src = brand(f"navia-app-icon-{variant}.svg")
    squircle = re.search(r'<path id="q" d="([^"]+)"/>', src).group(1)
    mark_def = re.search(r'<path id="m" d="[^"]+"/>', src).group(0)
    mark_use = re.search(r'<g transform="translate\(50 50\)[^>]*>.*?</g>', src, re.S).group(0)
    mark_grad_id = re.search(r'stroke="url\(#(\w+)\)"', mark_use).group(1)
    mark_grad = re.search(rf'<linearGradient id="{mark_grad_id}".*?</linearGradient>', src, re.S).group(0)
    if fill is not None:
        squircle = superellipse(DESKTOP_SQUIRCLE_N, (max_y(squircle) - min_y(squircle)) / 2)
    bg = BACKGROUND[variant]
    width = max_y(squircle) - min_y(squircle)
    inset = (width - RIM_WIDTH) / width
    rim_stops = "".join(f'<stop offset="{o:g}" stop-color="{bg["rim"]}" stop-opacity="{a:g}"/>' for o, a in bg["rim_stops"])
    # xlink:href, since older Qt SVG and librsvg may skip plain href.
    mark_use = mark_use.replace("<use href=", "<use xlink:href=")
    if fill is not None:
        stops = "".join(f'<stop offset="{o:g}" stop-color="{c}"/>' for o, c in DESKTOP_MARK_STOPS)
        mark_grad = re.sub(r'(<linearGradient id="\w+"[^>]*>).*?(</linearGradient>)', lambda m: m.group(1) + stops + m.group(2), mark_grad, count=1, flags=re.S)
        mark_use = re.sub(r'stroke-width="([\d.]+)"', lambda m: f'stroke-width="{float(m.group(1)) * DESKTOP_MARK_STROKE:g}"', mark_use, count=1)
        mark_use = re.sub(r'scale\(([\d.]+)\)', lambda m: f"scale({float(m.group(1)) * DESKTOP_MARK_SCALE:.4f})", mark_use, count=1)
    return ('<?xml version="1.0" encoding="UTF-8"?>\n'
            f'<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="{view_box(width, fill)}" width="1024" height="1024">'
            '<defs>'
            f'<linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{bg["top"]}"/><stop offset="1" stop-color="{bg["bottom"]}"/></linearGradient>'
            f'<linearGradient id="rim" gradientUnits="userSpaceOnUse" x1="0" y1="{min_y(squircle):g}" x2="0" y2="{max_y(squircle):g}">{rim_stops}</linearGradient>'
            f'{mark_grad}'
            f'<path id="q" d="{squircle}"/>{mark_def}'
            '</defs>'
            '<use xlink:href="#q" fill="url(#bg)"/>'
            # An inside stroke without clip-path, which KDE's Qt SVG ignores. The squircle is shrunk
            # so the stroke ends exactly at its edge.
            f'<use xlink:href="#q" transform="translate(50 50) scale({inset:.5f}) translate(-50 -50)" fill="none" stroke="url(#rim)" stroke-width="{RIM_WIDTH / inset:.4f}"/>'
            f'{mark_use}</svg>\n')


def tray_svg(color=None, px=TRAY_SVG_PX):
    """The bare tray mark, cropped to fill TRAY_FILL of the square. Without `color` it's a symbolic
    icon that Plasma recolors for its theme."""
    src = brand("navia-mark-white.svg")
    path = re.search(r' d="([^"]+)"', src).group(1)
    nums = [float(v) for v in re.findall(r"-?\d+(?:\.\d+)?", path)]
    reach = max(nums) - min(nums)
    # The box spans the line's outer edges, so stroke and box depend on each other.
    stroke = max(3.2, TRAY_LINE_PX * reach / (TRAY_FILL * px - TRAY_LINE_PX))
    side = (reach + stroke) / TRAY_FILL
    box = f"{50 - side / 2:.4f} {50 - side / 2:.4f} {side:.4f} {side:.4f}"
    if color is None:
        style = '<style id="current-color-scheme" type="text/css">.ColorScheme-Text { color:#232629; }</style>'
        paint = 'class="ColorScheme-Text" style="fill:none;stroke:currentColor"'
    else:
        style, paint = "", f'fill="none" stroke="{color}"'
    return ('<?xml version="1.0" encoding="UTF-8"?>\n'
            f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{box}" width="22" height="22">{style}'
            f'<path d="{path}" {paint} stroke-width="{stroke:.3f}" stroke-linejoin="round"/></svg>\n')


def mark_path():
    """The mark as a closed path of cubic Béziers in its 100-unit box."""
    h = 2 * math.pi / MARK_STEPS
    point = lambda t: (50 + MARK_R * math.sin(MARK_FX * t + MARK_PHASE), 50 + MARK_R * math.sin(MARK_FY * t))
    slope = lambda t: (MARK_R * MARK_FX * math.cos(MARK_FX * t + MARK_PHASE), MARK_R * MARK_FY * math.cos(MARK_FY * t))
    x, y = point(0)
    d = [f"M{x:.3f} {y:.3f}"]
    for i in range(MARK_STEPS):
        t0, t1 = i * h, (i + 1) * h
        (x0, y0), (x1, y1), (dx0, dy0), (dx1, dy1) = point(t0), point(t1), slope(t0), slope(t1)
        d.append(f"C{x0 + dx0 * h / 3:.3f} {y0 + dy0 * h / 3:.3f} {x1 - dx1 * h / 3:.3f} {y1 - dy1 * h / 3:.3f} {x1:.3f} {y1:.3f}")
    return "".join(d) + "Z"


def mark_length():
    """The mark's length in its box, for the draw-in's dash."""
    n = 20000
    pts = [(50 + MARK_R * math.sin(MARK_FX * t + MARK_PHASE), 50 + MARK_R * math.sin(MARK_FY * t)) for t in (2 * math.pi * i / n for i in range(n + 1))]
    return sum(math.dist(a, b) for a, b in zip(pts, pts[1:]))


def cubic_ease(x, curve):
    """CSS cubic-bezier(x1, y1, x2, y2) at progress x."""
    x1, y1, x2, y2 = curve
    bez = lambda t, a, b: 3 * a * (1 - t) ** 2 * t + 3 * b * (1 - t) * t ** 2 + t ** 3
    lo, hi = 0.0, 1.0
    for _ in range(60):
        mid = (lo + hi) / 2
        lo, hi = (mid, hi) if bez(mid, x1, x2) < x else (lo, mid)
    return bez((lo + hi) / 2, y1, y2)


def svg_png(svg):
    out = subprocess.run(["rsvg-convert", "-f", "png"], input=svg.encode(), capture_output=True, check=True)
    return out.stdout


_WORD = {}


def word_font():
    """Mona Sans at the wordmark's weight and width, as a static instance."""
    if "font" not in _WORD:
        import io
        from fontTools.ttLib import TTFont
        from fontTools.varLib import instancer
        font = instancer.instantiateVariableFont(TTFont(os.path.join(BRAND, BRAND_WORD_FONT)), BRAND_WORD_AXES)
        buf = io.BytesIO()
        font.save(buf)
        buf.seek(0)
        _WORD["font"] = TTFont(buf)
    return _WORD["font"]


def pair_kern(font, left, right):
    """The kerning between two glyphs in font units, from GPOS pair adjustments."""
    gpos = font["GPOS"].table if "GPOS" in font else None
    if gpos is None or not gpos.LookupList:
        return 0
    lookups = sorted({i for r in gpos.FeatureList.FeatureRecord if r.FeatureTag == "kern" for i in r.Feature.LookupListIndex})
    for i in lookups:
        lookup = gpos.LookupList.Lookup[i]
        for sub in lookup.SubTable:
            sub = sub.ExtSubTable if lookup.LookupType == 9 else sub
            if getattr(sub, "LookupType", lookup.LookupType) != 2 or left not in sub.Coverage.glyphs:
                continue
            if sub.Format == 1:
                for rec in sub.PairSet[sub.Coverage.glyphs.index(left)].PairValueRecord:
                    if rec.SecondGlyph == right:
                        return getattr(rec.Value1, "XAdvance", 0) or 0
            else:
                c1, c2 = sub.ClassDef1.classDefs.get(left, 0), sub.ClassDef2.classDefs.get(right, 0)
                value = sub.Class1Record[c1].Class2Record[c2].Value1
                if value is not None and getattr(value, "XAdvance", 0):
                    return value.XAdvance
    return 0


def word_outline():
    """The wordmark at 1 unit to the em, pen origin and baseline at 0, y down. Returns its SVG path
    and the right edge of its ink."""
    if "outline" not in _WORD:
        from fontTools.pens.boundsPen import BoundsPen
        from fontTools.pens.svgPathPen import SVGPathPen
        from fontTools.pens.transformPen import TransformPen
        font = word_font()
        glyphs, upm = font.getGlyphSet(), font["head"].unitsPerEm
        names = [font.getBestCmap()[ord(c)] for c in BRAND_WORD]
        pen, bounds, x = SVGPathPen(glyphs, ntos=lambda v: f"{v:.5f}".rstrip("0").rstrip(".")), BoundsPen(glyphs), 0.0
        for i, name in enumerate(names):
            k = (1 / upm, 0, 0, -1 / upm, x, 0)
            glyphs[name].draw(TransformPen(pen, k))
            glyphs[name].draw(TransformPen(bounds, k))
            if i + 1 < len(names):
                x += (font["hmtx"][name][0] + pair_kern(font, name, names[i + 1])) / upm + BRAND_WORD_TRACKING
        _WORD["outline"] = (pen.getCommands(), bounds.bounds[2])
    return _WORD["outline"]


def word_svg(x, y, size, opacity=1.0, fill=SPLASH_INK):
    """The wordmark as a path with its pen origin at x and its baseline at y."""
    d, _ = word_outline()
    return f'<path transform="translate({x:.3f} {y:.3f}) scale({size:g})" d="{d}" fill="{fill}" opacity="{opacity:.4f}"/>'


def x_height():
    """The wordmark's x-height as a fraction of the em, from the x's ink."""
    from fontTools.pens.boundsPen import BoundsPen
    font = word_font()
    glyphs = font.getGlyphSet()
    pen = BoundsPen(glyphs)
    glyphs[font.getBestCmap()[ord("x")]].draw(pen)
    return pen.bounds[3] / font["head"].unitsPerEm


def mark_box_for(ink):
    """The mark's 100-unit box size whose ink (the curve's reach plus the stroke) is `ink`."""
    return ink * 100 / (2 * MARK_R + BRAND_STROKE)


def lockup_layout(mark, cx, cy):
    """Lays out the lockup with its ink centered on (cx, cy). The mark's center sits on the middle of
    the x-height."""
    size = mark * BRAND_WORD_RATIO
    width = mark + mark * BRAND_GAP_RATIO + word_outline()[1] * size
    left = cx - width / 2
    box = mark_box_for(mark)
    mark_x, mark_y = left - (box - mark) / 2, cy - box / 2
    word_x = left + mark + mark * BRAND_GAP_RATIO
    baseline = cy + x_height() * size / 2
    return (mark_x, mark_y, box), (word_x, baseline, size)


def gpui_mark_svg():
    half = BRAND_STROKE / 2
    lo, span = 50 - MARK_R - half, 2 * MARK_R + BRAND_STROKE
    return ('<?xml version="1.0" encoding="UTF-8"?>\n'
            f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{lo:g} {lo:g} {span:g} {span:g}">'
            f'<path d="{mark_path()}" fill="none" stroke="#FFFFFF" stroke-width="{BRAND_STROKE:g}" stroke-linejoin="round"/></svg>\n')


def gpui_word_svg():
    """The wordmark as paths at 100 units to the em, in a box one em tall centered on the x-height."""
    size = 100
    right = word_outline()[1] * size
    mid = -x_height() * size / 2
    return ('<?xml version="1.0" encoding="UTF-8"?>\n'
            f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 {mid - size / 2:g} {right:g} {size}">'
            f'{word_svg(0, 0, size, fill="#FFFFFF")}</svg>\n')


def brand_wordmark_svg(color):
    """The brand's wordmark file: the letters as paths, the box their ink."""
    size, (d, right) = 100, word_outline()
    top, bottom = word_ink_y()
    return ('<?xml version="1.0" encoding="UTF-8"?>\n'
            f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 {top * size:g} {right * size:g} {(bottom - top) * size:g}">'
            f'{word_svg(0, 0, size, fill=color)}</svg>\n')


def word_ink_y():
    """The wordmark's ink from top to bottom, at 1 unit to the em, y down."""
    from fontTools.pens.boundsPen import BoundsPen
    font = word_font()
    glyphs, upm = font.getGlyphSet(), font["head"].unitsPerEm
    tops, bottoms = [], []
    for c in set(BRAND_WORD):
        pen = BoundsPen(glyphs)
        glyphs[font.getBestCmap()[ord(c)]].draw(pen)
        tops.append(-pen.bounds[3] / upm)
        bottoms.append(-pen.bounds[1] / upm)
    return min(tops), max(bottoms)


def brand_lockup_svg(color):
    """The brand's lockup file, with the box fitted to the ink of the mark and wordmark."""
    mark = 100
    (mark_x, mark_y, box), (word_x, baseline, size) = lockup_layout(mark, 0, 0)
    top, bottom = word_ink_y()
    left, right = mark_x + (box - mark) / 2, word_x + word_outline()[1] * size
    y0 = min(-mark / 2, baseline + top * size)
    y1 = max(mark / 2, baseline + bottom * size)
    return ('<?xml version="1.0" encoding="UTF-8"?>\n'
            f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{left:g} {y0:g} {right - left:g} {y1 - y0:g}">'
            f'<svg x="{mark_x:.3f}" y="{mark_y:.3f}" width="{box:.3f}" height="{box:.3f}" viewBox="0 0 100 100">'
            f'<path d="{mark_path()}" fill="none" stroke="{color}" stroke-width="{BRAND_STROKE:g}" stroke-linejoin="round"/></svg>'
            f'{word_svg(word_x, baseline, size, fill=color)}</svg>\n')


BAYER_8 = [[0, 32, 8, 40, 2, 34, 10, 42], [48, 16, 56, 24, 50, 18, 58, 26], [12, 44, 4, 36, 14, 46, 6, 38],
           [60, 28, 52, 20, 62, 30, 54, 22], [3, 35, 11, 43, 1, 33, 9, 41], [51, 19, 59, 27, 49, 17, 57, 25],
           [15, 47, 7, 39, 13, 45, 5, 37], [63, 31, 55, 23, 61, 29, 53, 21]]


def splash_background():
    """The dark icon gradient, dithered to 8 bits with an 8×8 Bayer pattern so it doesn't band."""
    import numpy as np
    src = brand("navia-app-icon-dark.svg")
    grad = re.search(rf'<linearGradient id="{SPLASH_BG_GRADIENT}".*?</linearGradient>', src, re.S).group(0)
    stops = [(float(o), c) for o, c in re.findall(r'<stop offset="([\d.]+)" stop-color="(#[0-9A-Fa-f]{6})"', grad)]
    rgb = lambda c: np.array([int(c[i:i + 2], 16) for i in (1, 3, 5)], dtype=np.float64)
    ys = (np.arange(SPLASH_H) + 0.5) / SPLASH_H
    rows = np.zeros((SPLASH_H, 3))
    for (o0, c0), (o1, c1) in zip(stops, stops[1:]):
        inside = (ys >= o0) & (ys <= o1)
        f = ((ys[inside] - o0) / max(o1 - o0, 1e-9))[:, None]
        rows[inside] = rgb(c0) * (1 - f) + rgb(c1) * f
    threshold = (np.array(BAYER_8, dtype=np.float64) + 0.5) / 64
    tiled = np.tile(threshold, (SPLASH_H // 8 + 1, SPLASH_W // 8 + 1))[:SPLASH_H, :SPLASH_W, None]
    return np.clip(np.floor(rows[:, None, :] + tiled), 0, 255)


def splash_frames():
    """The splash frames as 8-bit RGB arrays, with their durations in ms."""
    import io
    import numpy as np
    from PIL import Image
    background = splash_background()
    (mark_x, mark_y, box), (word_x, word_y, word_size) = lockup_layout(SPLASH_MARK, SPLASH_W / 2, SPLASH_H / 2)
    length = mark_length()
    path = mark_path()

    def frame(t):
        drawn = cubic_ease(min(t / SPLASH_DRAW_S, 1.0), SPLASH_DRAW_EASE)
        fade = cubic_ease(min(max((t - SPLASH_WORD_START_S) / SPLASH_WORD_S, 0.0), 1.0), SPLASH_WORD_EASE)
        mark = ""
        if drawn > 0:
            mark = (f'<svg x="{mark_x:.3f}" y="{mark_y:.3f}" width="{box:.3f}" height="{box:.3f}" viewBox="0 0 100 100">'
                    f'<path d="{path}" fill="none" stroke="{SPLASH_INK}" stroke-width="{BRAND_STROKE:g}" stroke-linecap="round" stroke-linejoin="round" '
                    f'stroke-dasharray="{length:.3f} {length:.3f}" stroke-dashoffset="{length * (1 - drawn):.3f}"/></svg>')
        word = word_svg(word_x, word_y + SPLASH_WORD_SLIDE * (1 - fade), word_size, fade) if fade > 0 else ""
        svg = f'<svg xmlns="http://www.w3.org/2000/svg" width="{SPLASH_W}" height="{SPLASH_H}" viewBox="0 0 {SPLASH_W} {SPLASH_H}">{mark}{word}</svg>'
        ink = np.asarray(Image.open(io.BytesIO(svg_png(svg))).convert("RGBA"), dtype=np.float64)
        alpha = ink[:, :, 3:] / 255
        return np.clip(np.rint(ink[:, :, :3] * alpha + background * (1 - alpha)), 0, 255).astype(np.uint8)

    end = max(SPLASH_DRAW_S, SPLASH_WORD_START_S + SPLASH_WORD_S)
    count = math.ceil(end * SPLASH_FPS)
    frames = [frame(i / SPLASH_FPS) for i in range(count + 1)]
    step = round(1000 / SPLASH_FPS)
    frames += [frames[-1]] * round(SPLASH_HOLD_S * SPLASH_FPS)
    return frames, [step] * len(frames)


def splash_files():
    """Writes the animated GIF and the last frame as PNG. Frames map exactly onto the palette, because
    PIL's own mapping rounds and shifts the gradient."""
    import io
    import numpy as np
    from PIL import Image
    frames, durations = splash_frames()
    flat = lambda a: a.reshape(-1, 3)
    background = {tuple(c) for c in np.unique(flat(frames[0]), axis=0)}
    bg_colors = sorted(background)
    room = 256 - len(bg_colors)
    last = Image.fromarray(frames[-1])
    extra = last.quantize(colors=room, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.NONE).getpalette()[:room * 3]
    palette = np.array(bg_colors + [tuple(extra[i:i + 3]) for i in range(0, len(extra), 3)], dtype=np.int64)
    indexed = []
    for f in frames:
        colors, inverse = np.unique(flat(f), axis=0, return_inverse=True)
        nearest = ((colors[:, None, :].astype(np.int64) - palette[None, :, :]) ** 2).sum(axis=2).argmin(axis=1)
        image = Image.fromarray(nearest[inverse.reshape(-1)].reshape(SPLASH_H, SPLASH_W).astype(np.uint8), mode="P")
        image.putpalette([int(v) for v in palette.reshape(-1)] + [0] * (768 - palette.size))
        indexed.append(image)
    # Pillow merges identical frames, which Velopack would average into a slow frame rate, so every
    # other held frame differs by one corner pixel.
    held = round(SPLASH_HOLD_S * SPLASH_FPS)
    for k, image in enumerate(indexed[len(indexed) - held:]):
        if k % 2 == 0:
            image.putpixel((0, 0), (image.getpixel((0, 0)) + 1) % len(bg_colors))
    gif = io.BytesIO()
    indexed[0].save(gif, format="GIF", save_all=True, append_images=indexed[1:], duration=durations, loop=0, optimize=False, disposal=1)
    png = io.BytesIO()
    last.save(png, format="PNG", optimize=True)
    return gif.getvalue(), png.getvalue()


def superellipse(n, radius, steps=720):
    """A closed superellipse |x|^n + |y|^n = r^n centered on the 100-unit canvas, as an SVG path."""
    pts = []
    for i in range(steps):
        t = 2 * math.pi * i / steps
        c, s_ = math.cos(t), math.sin(t)
        pts.append(f"{50 + radius * math.copysign(abs(c) ** (2 / n), c):.3f} {50 + radius * math.copysign(abs(s_) ** (2 / n), s_):.3f}")
    return "M" + " ".join(pts) + "Z"  # later pairs after M are line-tos, which min_y/max_y also read


def view_box(width, fill):
    """The whole 100-unit canvas, or a square around the centred container so it fills `fill` of it."""
    if fill is None:
        return "0 0 100 100"
    side = width / fill
    return f"{50 - side / 2:.4f} {50 - side / 2:.4f} {side:.4f} {side:.4f}"


def _ys(path):
    return [float(v) for v in path.replace("M", " ").replace("Z", " ").split()[1::2]]


def min_y(path):
    return min(_ys(path))


def max_y(path):
    return max(_ys(path))


def ico(images):
    """A Windows .ico with one PNG-compressed image per size (Vista and later read these)."""
    head = bytearray(b"\0\0\1\0" + len(images).to_bytes(2, "little"))
    offset = 6 + 16 * len(images)
    body = bytearray()
    for size, png in images:
        dim = 0 if size >= 256 else size
        head += bytes([dim, dim, 0, 0]) + (1).to_bytes(2, "little") + (32).to_bytes(2, "little")
        head += len(png).to_bytes(4, "little") + (offset + len(body)).to_bytes(4, "little")
        body += png
    return bytes(head + body)


def outputs():
    files = {}
    files[os.path.join(MAC_ICON, "icon.json")] = (json.dumps(mac_icon_json(), indent=2) + "\n").encode()
    files[os.path.join(MAC_ICON, "Assets/mark-black.svg")] = mac_mark_svg("navia-mark-black.svg").encode()
    files[os.path.join(MAC_ICON, "Assets/mark-white.svg")] = mac_mark_svg("navia-mark-white.svg").encode()

    dark, light = container_svg("dark", DESKTOP_ICON_FILL).encode(), container_svg("light", DESKTOP_ICON_FILL).encode()
    files[os.path.join(LINUX, "app.svg")] = dark
    files[os.path.join(LINUX, "app-light.svg")] = light
    for size in LINUX_SIZES:
        files[os.path.join(LINUX, f"app-{size}.png")] = render_png(dark, size)
    files[WINDOWS_ICO] = ico([(s, render_png(dark, s)) for s in ICO_SIZES])
    files[WINDOWS_ICO.replace("app.ico", "app-light.ico")] = ico([(s, render_png(light, s)) for s in ICO_SIZES])
    files[WINDOWS_SPLASH_GIF], files[WINDOWS_SPLASH_PNG] = splash_files()
    files[GPUI_MARK] = gpui_mark_svg().encode()
    files[GPUI_WORD] = gpui_word_svg().encode()
    for name, color in (("white", "#FFFFFF"), ("black", "#000000")):
        files[os.path.join(BRAND, f"navia-wordmark-{name}.svg")] = brand_wordmark_svg(color).encode()
        files[os.path.join(BRAND, f"navia-lockup-horizontal-{name}.svg")] = brand_lockup_svg(color).encode()
    files[os.path.join(TRAY, "com.davekai.Navia.tray-symbolic.svg")] = tray_svg().encode()
    for name, color in (("white", "#FFFFFF"), ("black", "#000000")):
        for size in TRAY_SIZES:
            files[os.path.join(TRAY, f"mark-{name}-{size}.png")] = render_png(tray_svg(color, size).encode(), size)
    return files


def main():
    check = sys.argv[1:] == ["check"]
    stale = []
    for path, data in outputs().items():
        current = open(path, "rb").read() if os.path.exists(path) else None
        if current == data:
            continue
        stale.append(os.path.relpath(path, ROOT))
        if not check:
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "wb") as f:
                f.write(data)
    for p in stale:
        print(("stale: " if check else "wrote: ") + p)
    sys.exit(1 if check and stale else 0)


if __name__ == "__main__":
    main()
