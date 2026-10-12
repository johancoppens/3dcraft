import json, math, sys
m, alpha, N = 2.0, math.radians(20), 6
steps = [["variable.set", {"name": n, "expr": e}] for n, e in
         [("bore_d", "15 mm"), ("key_b", "5 mm"), ("key_t2", "2.3 mm"), ("face_w", "20 mm")]]
polys = {}
def gear(name, z, cx, rot):
    S = name + "Profile"
    r = m*z/2; ra = r+m; rf = r-1.25*m; rb = r*math.cos(alpha)
    psi_b = math.pi/(2*z) + math.tan(alpha)-alpha
    th_a = math.sqrt((ra/rb)**2-1)
    th_f = math.sqrt((rf/rb)**2-1) if rf > rb else 0.0
    pol = lambda rad, ang: [cx+rad*math.cos(ang), rad*math.sin(ang)]
    def flank(g, side):
        pts = [pol(rf, g+side*psi_b)] if rf < rb else []   # radial stub below the base circle
        for i in range(N+1):
            th = th_f+(th_a-th_f)*i/N
            pts.append(pol(rb*math.sqrt(1+th*th), g+side*(psi_b-(th-math.atan(th)))))
        return pts
    steps.append(["sketch.create", {"plane": "xy", "name": S}])
    steps.append(["sketch.point", {"sketch": S, "at": [cx, 0]}])
    P = len(flank(0, 1)); per = 2*P-1
    first = lambda j: 2+per*j
    last = lambda j: 2+per*j+P-1
    outline, area = [], 0.0
    def arcpts(a, b, rad, n=8):
        a0 = math.atan2(a[1], a[0]-cx); a1 = math.atan2(b[1], b[0]-cx)
        if a1 < a0: a1 += 2*math.pi
        return [pol(rad, a0+(a1-a0)*i/n) for i in range(1, n)]
    for k in range(z):
        g = rot+2*math.pi*k/z
        R = flank(g, -1); L = flank(g, +1)[::-1]
        steps.append(["sketch.polyline", {"sketch": S, "points": R}])
        steps.append(["sketch.polyline", {"sketch": S, "points": L}])
        nxt = flank(rot+2*math.pi*(k+1)/z, -1)[0]
        outline += R + arcpts(R[-1], L[0], ra) + L + arcpts(L[-1], nxt, rf)
        for seq in (R, L):
            for a, b in zip(seq, seq[1:]): area += ((a[0]-cx)*b[1]-(b[0]-cx)*a[1])/2
        for a, b, rad in ((R[-1], L[0], ra), (L[-1], nxt, rf)):
            area += ((a[0]-cx)*b[1]-(b[0]-cx)*a[1])/2
            ang = 2*math.asin(math.dist(a, b)/(2*rad))
            area += rad*rad*(ang-math.sin(ang))/2
    for k in range(z):
        steps.append(["sketch.arc", {"sketch": S, "center": "e1", "start": f"e{last(2*k)}", "end": f"e{first(2*k+1)}"}])
        steps.append(["sketch.arc", {"sketch": S, "center": "e1", "start": f"e{last(2*k+1)}", "end": f"e{first(2*((k+1)%z))}"}])
    n = 2+per*2*z+2*z
    d, b, t2 = 15.0, 5.0, 2.3
    y0 = math.sqrt((d/2)**2-(b/2)**2)
    steps.append(["sketch.polyline", {"sketch": S, "points": [[cx+b/2, y0], [cx+b/2, d/2+t2], [cx-b/2, d/2+t2], [cx-b/2, y0]]}])
    p = [f"e{n+i}" for i in range(4)]; l = [f"e{n+4+i}" for i in range(3)]; arc = f"e{n+7}"
    steps.append(["sketch.arc", {"sketch": S, "center": "e1", "start": p[3], "end": p[0]}])
    C = lambda **kw: steps.append(["sketch.constrain", dict(sketch=S, **kw)])
    C(type="fixed", entities=["e1"])
    C(type="radius", entities=[arc], value="bore_d / 2")
    C(type="distance", entities=["e1", p[0]], value="key_b / 2", direction="horizontal")
    C(type="distance", entities=["e1", p[3]], value="key_b / 2", direction="horizontal")
    C(type="vertical", entities=[l[0]]); C(type="horizontal", entities=[l[1]]); C(type="vertical", entities=[l[2]])
    C(type="distance", entities=["e1", p[1]], value="bore_d / 2 + key_t2", direction="vertical")
    steps.append(["feature.extrude", {"sketch": S, "distance": "face_w", "name": name}])
    half = math.asin((b/2)/(d/2))
    bore = (d/2)**2*(math.pi-half) + (b/2)*y0 + b*(d/2+t2-y0)
    polys[name] = outline
    print(name, "z", z, "expected volume", (area-bore)*20)
a = m*(30+50)/2
gear("Pinion", 30, 0.0, 0.0)             # a tooth points at the wheel
gear("Wheel", 50, a, math.pi+math.pi/50)  # a tooth space points at the pinion
steps += [["document.check", {}], ["document.measure", {}]]
json.dump(steps, open(sys.argv[1], "w"))
print("centre distance", a, "steps", len(steps))
try:
    from shapely.geometry import Polygon
    A, B = Polygon(polys["Pinion"]), Polygon(polys["Wheel"])
    print("valid", A.is_valid, B.is_valid, "overlap area", A.intersection(B).area, "min gap", A.distance(B))
except ImportError:
    print("shapely missing")
json.dump(polys, open(sys.argv[1]+".outline", "w"))
