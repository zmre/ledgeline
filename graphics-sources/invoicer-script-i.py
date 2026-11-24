"""Draws Ledgeline Invoicer's script "i" as SVG path data, so the logo needs no font.

Each stroke is a cubic-Bezier centreline swept by a pen: an elliptical
broad nib at 35 degrees for the brush variants, a round pen for "mono".

    python3 graphics-sources/invoicer-script-i.py brush 845 890 215

prints the `d` of the "i" (variant, centre x, baseline y, x-height, in the
logo's 1140.47 viewBox units). The logo uses `brush 845 890 215`; paste the
output into graphics-sources/ledgeline-invoicer-logo-no-word{,-no-bg}.svg
and assets/LedgelineInvoicer.icon/Assets/i.svg, then run `just icons`.
"""
import math, sys

def bez(p, t):
    a,b,c,d = p; mt = 1-t
    return tuple(mt**3*a[i] + 3*mt*mt*t*b[i] + 3*mt*t*t*c[i] + t**3*d[i] for i in (0,1))

def sample(chain, n):
    pts = []
    for k, seg in enumerate(chain):
        for j in range(n + (1 if k == len(chain)-1 else 0)):
            pts.append(bez(seg, j/n))
    return pts

NIB = None
def outline(pts, width, taper0, taper1):
    """width(phi) -> pen width for direction phi; taper fractions shrink the ends."""
    N = len(pts); L = [0.0]
    for i in range(1, N): L.append(L[-1] + math.dist(pts[i], pts[i-1]))
    tot = L[-1]; left = []; right = []; ws = []
    for i, p in enumerate(pts):
        a = pts[max(i-1, 0)]; b = pts[min(i+1, N-1)]
        phi = math.atan2(b[1]-a[1], b[0]-a[0])
        w = width(phi); s = L[i]/tot
        if taper0 and s < taper0: w *= 0.35 + 0.65*math.sin(s/taper0*math.pi/2)
        if taper1 and s > 1-taper1: w *= 0.35 + 0.65*math.sin((1-s)/taper1*math.pi/2)
        nx, ny = -math.sin(phi), math.cos(phi)
        if NIB:  # elliptical nib: the support point of the ellipse along the normal
            th, A, B = NIB; k = w/width(phi)  # taper scales the nib
            c, sn = math.cos(th), math.sin(th)
            u, v = nx*c+ny*sn, -nx*sn+ny*c        # normal in nib axes
            h = math.hypot(A*u, B*v)
            ox, oy = (A*A*u)/h, (B*B*v)/h        # support point in nib axes
            dx, dy = (ox*c - oy*sn)*k, (ox*sn + oy*c)*k
            left.append((p[0]+dx, p[1]+dy)); right.append((p[0]-dx, p[1]-dy)); ws.append((2*math.hypot(dx,dy), phi))
        else:
            left.append((p[0]+nx*w/2, p[1]+ny*w/2)); right.append((p[0]-nx*w/2, p[1]-ny*w/2)); ws.append((w, phi))
    left, right = unloop(left), unloop(right)
    def cap(c, w, phi, sgn):
        return [(c[0]+w/2*math.cos(phi+sgn*(math.pi/2 - math.pi*k/12)), c[1]+w/2*math.sin(phi+sgn*(math.pi/2 - math.pi*k/12))) for k in range(1, 12)]
    def ecap(c, k, phi, fwd):
        """Half of the (scaled) nib ellipse from the left support point to the
        right one, round the side facing `fwd` (the stroke's direction there)."""
        th, A, B = NIB; ct, st = math.cos(th), math.sin(th)
        nx, ny = -math.sin(phi), math.cos(phi)
        u, v = nx*ct+ny*st, -nx*st+ny*ct
        t0 = math.atan2(B*v, A*u)
        P = lambda t: (c[0]+k*(A*math.cos(t)*ct - B*math.sin(t)*st), c[1]+k*(A*math.cos(t)*st + B*math.sin(t)*ct))
        m = P(t0 - math.pi/2); sgn = -1 if ((m[0]-c[0])*fwd[0] + (m[1]-c[1])*fwd[1]) > 0 else 1
        return [P(t0 + sgn*math.pi*j/16) for j in range(1, 16)]
    w1, p1 = ws[-1]; w0, p0 = ws[0]
    if NIB:
        # scale factor = ratio of actual offset to full nib offset
        def kscale(i, phi):
            th, A, B = NIB; nx, ny = -math.sin(phi), math.cos(phi)
            ct, st = math.cos(th), math.sin(th); u, v = nx*ct+ny*st, -nx*st+ny*ct
            full = math.hypot(A*A*u, B*B*v)/math.hypot(A*u, B*v)
            return (ws[i][0]/2)/full
        poly = (left + ecap(pts[-1], kscale(-1, p1), p1, (math.cos(p1), math.sin(p1))) + right[::-1]
                + ecap(pts[0], kscale(0, p0), p0 + math.pi, (-math.cos(p0), -math.sin(p0))))
    else:
        poly = left + cap(pts[-1], w1, p1, 1) + right[::-1] + cap(pts[0], w0, p0 + math.pi, 1)
    return poly

def _seg_x(p, q, r, s):
    d = (q[0]-p[0])*(s[1]-r[1]) - (q[1]-p[1])*(s[0]-r[0])
    if abs(d) < 1e-12: return None
    t = ((r[0]-p[0])*(s[1]-r[1]) - (r[1]-p[1])*(s[0]-r[0]))/d
    u = ((r[0]-p[0])*(q[1]-p[1]) - (r[1]-p[1])*(q[0]-p[0]))/d
    return (p[0]+t*(q[0]-p[0]), p[1]+t*(q[1]-p[1])) if 0 < t < 1 and 0 < u < 1 else None

def unloop(pl):
    """Cut the swallowtail loops an offset curve makes on the inside of a turn
    tighter than half the pen width."""
    i = 0
    while i < len(pl)-1:
        for j in range(len(pl)-2, i+1, -1):
            x = _seg_x(pl[i], pl[i+1], pl[j], pl[j+1])
            if x:
                pl = pl[:i+1] + [x] + pl[j+1:]
                break
        i += 1
    return pl

def area(poly):
    return sum(poly[i][0]*poly[i-1][1] - poly[i-1][0]*poly[i][1] for i in range(len(poly)))/2

def ellipse(c, rx, ry, rot, n=48):
    r = math.radians(rot)
    return [(c[0]+rx*math.cos(t)*math.cos(r)-ry*math.sin(t)*math.sin(r), c[1]+rx*math.cos(t)*math.sin(r)+ry*math.sin(t)*math.cos(r)) for t in (2*math.pi*k/n for k in range(n))]

def to_path(polys, xform):
    out = []
    for poly in polys:
        q = [xform(p) for p in poly]
        if area(q) < 0: q = q[::-1]
        out.append("M" + "L".join(f"{x:.1f},{y:.1f}" for x, y in q) + "Z")
    return "".join(out)

V = sys.argv[1]
# Local units: baseline y=0, x-height 1, y up. Slant applied afterwards.
if V == "brush":      # broad-nib calligraphy: thick downstroke, thinner hairlines
    nib_angle, W, r = math.radians(35), 0.32, 0.36
    NIB = (nib_angle, W/2, W*r/2); pen = lambda phi: 1.0
    entry = [[(-0.62,0.04),(-0.40,0.18),(-0.12,0.55),(0.0,1.0)]]
    down  = [[(0.0,1.0),(-0.01,0.72),(-0.03,0.50),(-0.03,0.30)],
             [(-0.03,0.30),(-0.03,0.04),(0.24,-0.06),(0.52,0.30)]]
    strokes = [outline(sample(entry,90), pen, 0.35, 0), outline(sample(down,90), pen, 0, 0.25)]
    dot = [ellipse((0.17,1.42), 0.13, 0.10, 35)]
    slant = 14
elif V == "mono":     # monoline script, round ends: the boldest at small sizes
    pen = lambda phi: 0.25
    entry = [[(-0.58,0.10),(-0.38,0.22),(-0.12,0.56),(0.0,1.0)]]
    down  = [[(0.0,1.0),(-0.01,0.72),(-0.03,0.52),(-0.03,0.32)],
             [(-0.03,0.32),(-0.03,0.04),(0.26,-0.06),(0.52,0.28)]]
    strokes = [outline(sample(entry,60), pen, 0, 0), outline(sample(down,60), pen, 0, 0)]
    dot = [ellipse((0.15,1.42), 0.14, 0.14, 0)]
    slant = 12
elif V == "brushtail":  # brush downstroke with a curled tail, no entry stroke
    nib_angle, W, r = math.radians(35), 0.34, 0.45
    NIB = (nib_angle, W/2, W*r/2); pen = lambda phi: 1.0
    down  = [[(-0.06,1.0),(-0.02,0.72),(-0.03,0.50),(-0.03,0.30)],
             [(-0.03,0.30),(-0.03,0.04),(0.26,-0.06),(0.55,0.32)]]
    strokes = [outline(sample(down,90), pen, 0, 0.25)]
    dot = [ellipse((0.12,1.42), 0.15, 0.11, 35)]
    slant = 14
cx, base, xh = [float(a) for a in sys.argv[2:5]]
t = math.tan(math.radians(slant))
xform = lambda p: (cx + (p[0] + p[1]*t)*xh, base - p[1]*xh)
print(to_path(strokes + dot, xform))
