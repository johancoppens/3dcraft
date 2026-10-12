import json, math, sys
m, z, alpha = 2.0, 30, math.radians(20)
N = 6
r = m*z/2; ra = r+m; rf = r-1.25*m; rb = r*math.cos(alpha)
psi_b = math.pi/(2*z) + math.tan(alpha)-alpha
th_a = math.sqrt((ra/rb)**2-1)
def pol(rad, ang): return [rad*math.cos(ang), rad*math.sin(ang)]
def flank(gamma, side):  # side -1 right, +1 left; root -> tip
    pts = [pol(rf, gamma+side*psi_b)]
    for i in range(N+1):
        th = th_a*i/N
        pts.append(pol(rb*math.sqrt(1+th*th), gamma+side*(psi_b-(th-math.atan(th)))))
    return pts
S = "Profile"
steps = [["variable.set", {"name": n, "expr": e}] for n, e in
         [("bore_d", "15 mm"), ("key_b", "5 mm"), ("key_t2", "2.3 mm"), ("face_w", "20 mm")]]
steps.append(["sketch.create", {"plane": "xy", "name": S}])
steps.append(["sketch.point", {"sketch": S, "at": [0, 0]}])
P = N+2; per = 2*P-1
first = lambda j: 2+per*j
last = lambda j: 2+per*j+P-1
area = 0.0
for k in range(z):
    g = 2*math.pi*k/z
    R = flank(g, -1); L = flank(g, +1)[::-1]
    steps.append(["sketch.polyline", {"sketch": S, "points": R}])
    steps.append(["sketch.polyline", {"sketch": S, "points": L}])
    # exact area: shoelace over chords + circular segments of tip and root arcs
    for seq in (R, L):
        for a, b in zip(seq, seq[1:]): area += (a[0]*b[1]-b[0]*a[1])/2
    g2 = 2*math.pi*(k+1)/z
    nxt = flank(g2, -1)[0]
    for a, b, rad in ((R[-1], L[0], ra), (L[-1], nxt, rf)):
        area += (a[0]*b[1]-b[0]*a[1])/2
        ang = 2*math.asin(math.dist(a, b)/(2*rad))
        area += rad*rad*(ang-math.sin(ang))/2
for k in range(z):
    steps.append(["sketch.arc", {"sketch": S, "center": "e1", "start": f"e{last(2*k)}", "end": f"e{first(2*k+1)}"}])
    steps.append(["sketch.arc", {"sketch": S, "center": "e1", "start": f"e{last(2*k+1)}", "end": f"e{first(2*((k+1)%z))}"}])
n = 2+per*2*z+2*z   # next id after the arcs
d, b, t2 = 15.0, 5.0, 2.3
y0 = math.sqrt((d/2)**2-(b/2)**2)
steps.append(["sketch.polyline", {"sketch": S, "points": [[b/2, y0], [b/2, d/2+t2], [-b/2, d/2+t2], [-b/2, y0]]}])
p = [f"e{n+i}" for i in range(4)]; l = [f"e{n+4+i}" for i in range(3)]; arc = f"e{n+7}"
steps.append(["sketch.arc", {"sketch": S, "center": "e1", "start": p[3], "end": p[0]}])
C = lambda **kw: steps.append(["sketch.constrain", dict(sketch=S, **kw)])
C(type="fixed", entities=["e1"])
C(type="radius", entities=[arc], value="bore_d / 2")
C(type="distance", entities=["e1", p[0]], value="key_b / 2", direction="horizontal")
C(type="distance", entities=["e1", p[3]], value="key_b / 2", direction="horizontal")
C(type="vertical", entities=[l[0]])
C(type="horizontal", entities=[l[1]])
C(type="vertical", entities=[l[2]])
C(type="distance", entities=["e1", p[1]], value="bore_d / 2 + key_t2", direction="vertical")
steps.append(["feature.extrude", {"sketch": S, "distance": "face_w", "name": "Gear"}])
steps.append(["document.check", {}])
steps.append(["document.measure", {}])
json.dump(steps, open(sys.argv[1], "w"))
half = math.asin((b/2)/(d/2))
bore = (d/2)**2*(math.pi-half) + (b/2)*y0 + b*(d/2+t2-y0)   # circle sector + triangles + slot rectangle
print("ra rf rb", ra, rf, rb, "tip width", 2*(psi_b-(th_a-math.atan(th_a)))*ra)
print("outline area", area, "bore+keyway area", bore, "expected volume", (area-bore)*20)
