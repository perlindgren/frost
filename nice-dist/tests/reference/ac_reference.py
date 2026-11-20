"""Independent complex-frequency nodal reference for the documented nominal model.
No Rust model code is used. Run with Python 3 to regenerate ac.csv.
The bilinear frequency matches trapezoidal capacitor companions at 384 kHz.
"""
import cmath
import csv
from pathlib import Path
RATE = 384000

def solve(a, b):
    for k in range(len(b)):
        pivot = max(range(k, len(b)), key=lambda r: abs(a[r][k]))
        a[k], a[pivot] = a[pivot], a[k]
        b[k], b[pivot] = b[pivot], b[k]
        d = a[k][k]
        a[k] = [x / d for x in a[k]]
        b[k] /= d
        for r in range(len(b)):
            if r == k:
                continue
            d = a[r][k]
            a[r] = [x - d*y for x, y in zip(a[r], a[k])]
            b[r] -= d*b[k]
    return b

def network(size, edges, sources, constraint=None):
    a = [[0j]*size for _ in range(size)]
    b = [0j]*size
    for p,q,g in edges:
        for i,j in [(p,q), (q,p)]:
            if i < 0:
                continue
            a[i][i] += g
            if j >= 0:
                a[i][j] -= g
    for node, g in sources:
        a[node][node] += g
        b[node] += g
    if constraint:
        a[-1] = constraint
        b[-1] = 0
    return solve(a,b)

def response(f, drive, tone, level, model):
    od1 = model == 3
    sd1 = model in (2,3)
    z = cmath.exp(-2j*cmath.pi*f/RATE)
    s = 2*RATE*(1-z)/(1+z)
    if model == 4:
        # Independent complex small-signal RAT, including backward-Euler op-amp pole.
        gain = network(2, [(0,-1,1/1e6),(0,1,1/1000),(1,-1,s*1e-9)], [(0,s*22e-9)])[1]
        zf = 1/(1/max(.1,100000*drive**2)+s*100e-12)
        yg = 1/(47+1/(s*2.2e-6))+1/(560+1/(s*4.7e-6))
        a = (2*cmath.pi*1e6)/(RATE*(1-z)+2*cmath.pi*1e6/100000)
        gain *= a*(1+zf*yg)/(a+1+zf*yg)
        edges = [(0,1,1/1000),(1,-1,2*2.52e-9/.045),
                 (1,2,1/(1500+100000*tone**2)),(2,-1,s*3.3e-9),
                 (2,3,s*22e-9),(3,-1,1/1e6)]
        gain *= network(4,edges,[(0,s*4.7e-6)])[3]
        edges = [(0,1,1/max(.1,100000*(1-level**2))),
                 (1,-1,1/max(.1,100000*level**2)+1/1e6)]
        return gain * network(2,edges,[(0,s*1e-6)])[1]
    hp = lambda c, rs, rl: s*c*rl / (1+s*c*(rs+rl))
    gain = hp(47e-9 if sd1 else 22e-9,1000 if od1 else (10000 if sd1 else 1000),470000 if sd1 else 510000)
    gain *= hp(4.7e-9 if od1 else (18e-9 if sd1 else 1e-6),0,100000 if sd1 else 10000)
    resistance = 33000+1000000*drive if sd1 else 51000+500000*drive**2
    diode_g = (1.5 if sd1 else 2)*2.52e-9/.045
    zf = 1/(1/resistance+(0 if sd1 else s*51e-12)+diode_g)
    gain *= 1+zf/(4700+1/(s*47e-9))
    gleft = 1/max(.1,20000*tone)
    gright = 1/max(.1,20000*(1-tone))
    cpositive, cwiper = (18e-9,27e-9) if sd1 else (220e-9,220e-9)
    edges = [(0,-1,s*cpositive),(0,1,gleft),(1,3,gright),(1,2,s*cwiper),(2,-1,1/(470 if sd1 else 220)),(3,4,1/(10000 if sd1 else 1000)+(s*10e-9 if sd1 else 0))]
    if not sd1:
        edges.append((0,-1,1/10000))
    nodes = network(5,edges,[(0,1/(10000 if sd1 else 1000))],[-1,0,0,1,0])
    gain *= -1/(1+s*10000*18e-9) if od1 else nodes[4]
    pos = level if sd1 else level**2
    pot = 10000 if sd1 else 100000
    edges = [(0,1,1/(4700 if sd1 else 1000)),(1,2,1/max(.1,pot*(1-pos))),(2,-1,1/max(.1,pot*pos)),(2,3,1/(22000 if sd1 and not od1 else .1)),(3,4,s*(47e-9 if sd1 else 100e-9)),(4,-1,1/(1000000 if sd1 else 510000))]
    if sd1 and not od1:
        edges.append((3,-1,1/22000))
    nodes = network(5,edges,[(0,s*1e-6)])
    gain *= nodes[4]
    shunt = 10000 if model == 0 else 100000
    series = [100,470,1000,1000][model]
    load = 1/(1/shunt+1/1000000)
    return gain*hp(1e-6 if sd1 else 10e-6,series,load)

with Path(__file__).with_name('ac.csv').open('w') as file:
    w = csv.writer(file)
    w.writerow(['frequency','drive','tone','level','model','real','imag'])
    for model in [0,1,2,3,4]:
        for drive,tone in [(0,.2),(.5,.5),(1,.8)]:
            for f in [375,1500,6000]:
                h = response(f,drive,tone,.7,model)
                w.writerow([f,drive,tone,.7,model,h.real,h.imag])
