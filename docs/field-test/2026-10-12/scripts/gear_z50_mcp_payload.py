import json, math, sys
m, z, alpha, N = 2.0, 50, math.radians(20), 6
r = m*z/2; ra = r+m; rf = r-1.25*m; rb = r*math.cos(alpha)
psi_b = math.pi/(2*z) + math.tan(alpha)-alpha
th_a = math.sqrt((ra/rb)**2-1)
th_f = math.sqrt((rf/rb)**2-1)  # base circle lies below the root circle for z >= 42
def pol(rad, ang): return [round(rad*math.cos(ang), 8), round(rad*math.sin(ang), 8)]
def flank(gamma, side):
    pts = []
    for i in range(N+1):
        th = th_f+(th_a-th_f)*i/N
        pts.append(pol(rb*math.sqrt(1+th*th), gamma+side*(psi_b-(th-math.atan(th)))))
    return pts
S = "F1"
P = N+1; per = 2*P-1
first = lambda j: 2+per*j
last = lambda j: 2+per*j+P-1
polys, arcs, area = [], [], 0.0
for k in range(z):
    g = 2*math.pi*k/z
    R = flank(g, -1); L = flank(g, +1)[::-1]
    polys.append({"id": "sketch.polyline", "params": {"sketch": S, "points": R}})
    polys.append({"id": "sketch.polyline", "params": {"sketch": S, "points": L}})
    for seq in (R, L):
        for a, b in zip(seq, seq[1:]): area += (a[0]*b[1]-b[0]*a[1])/2
    nxt = flank(2*math.pi*(k+1)/z, -1)[0]
    for a, b, rad in ((R[-1], L[0], ra), (L[-1], nxt, rf)):
        area += (a[0]*b[1]-b[0]*a[1])/2
        ang = 2*math.asin(math.dist(a, b)/(2*rad))
        area += rad*rad*(ang-math.sin(ang))/2
    arcs.append({"id": "sketch.arc", "params": {"sketch": S, "center": "e1", "start": f"e{last(2*k)}", "end": f"e{first(2*k+1)}"}})
    arcs.append({"id": "sketch.arc", "params": {"sketch": S, "center": "e1", "start": f"e{last(2*k+1)}", "end": f"e{first(2*((k+1)%z))}"}})
n = 2+per*2*z+2*z
d, b, t2 = 15.0, 5.0, 2.3
y0 = math.sqrt((d/2)**2-(b/2)**2)
half = math.asin((b/2)/(d/2))
bore = (d/2)**2*(math.pi-half) + (b/2)*y0 + b*(d/2+t2-y0)
dump = lambda x: json.dumps(x, separators=(",", ":"))
w = sys.argv[1]
if w == "a1": print(dump(polys[:50]))
if w == "a2": print(dump(polys[50:]))
if w == "b": print(dump(arcs)); print("next id", n, "y0", y0)
if w == "info": print("ra rf rb", ra, rf, rb, "outline", area, "bore", bore, "volume", (area-bore)*20)
