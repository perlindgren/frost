"""Independent full-circuit AC reference; Python standard library only.

Run from any directory to regenerate ac.csv beside this script. This builds a
single modified nodal matrix directly from the April 2009 Chittum trace. It does
not import, translate, or invoke the Rust solver or its stage partitioning.
The diode pair is linearized at zero bias. The frequency is bilinear-warped
so this isolates circuit discretization from the separate resampling filters.
"""
import cmath
import math
from pathlib import Path


def solve(a, b):
    for col in range(len(b)):
        pivot = max(range(col, len(b)), key=lambda row: abs(a[row][col]))
        a[col], a[pivot] = a[pivot], a[col]
        b[col], b[pivot] = b[pivot], b[col]
        d = a[col][col]
        a[col] = [v / d for v in a[col]]
        b[col] /= d
        for row in range(len(b)):
            if row == col:
                continue
            d = a[row][col]
            a[row] = [v - d * p for v, p in zip(a[row], a[col])]
            b[row] -= d * b[col]
    return b


def network(s, gain, treble, output):
    # All node numbers here correspond to the test's documented node map.
    size = 27
    a = [[0j] * size for _ in range(size)]
    b = [0j] * size
    capacitors = []

    def adm(p, q, y):
        if p >= 0:
            a[p][p] += y
        if q >= 0:
            a[q][q] += y
        if p >= 0 and q >= 0:
            a[p][q] -= y
            a[q][p] -= y

    def r(p, q, ohms):
        adm(p, q, 1 / max(0.001, ohms))

    def c(p, q, farads):
        adm(p, q, s * farads)
        capacitors.append((p, q, farads))

    # Buffered source: C1, R1, R2. U1A is ideal in small-signal AC.
    a[0][23] = 1
    a[23][0] = 1
    b[23] = 0.1 * s / (1 + 0.101 * s)
    # U1B is finite-bandwidth; U2A/U2B are ideal. Output-current unknowns.
    for equation, plus, minus, out in [(24, 2, 3, 5), (25, -1, 15, 16), (26, -1, 17, 19)]:
        a[out][equation] = 1
        if plus >= 0:
            a[equation][plus] = 1
        a[equation][minus] = -1

    a[24][5] = -(s + 2*math.pi*3e6/200e3) / (2*math.pi*3e6)

    c(0, 1, 100e-9)  # C3
    r(1, 2, 10e3); c(1, 2, 68e-9)  # R6,C5
    r(2, -1, gain * 100e3)  # P1B upper
    r(4, -1, 2e3 + (1 - gain) * 100e3)  # R10 + P1B lower
    r(4, 3, 15e3); c(4, 3, 82e-9)  # R11,C7
    r(3, 5, 422e3); c(3, 5, 390e-12)  # R12,C8
    r(1, 6, 1.5e3); c(6, -1, 1e-6); r(6, 15, 15e3)  # R7,C16,R19
    c(5, 7, 1e-6); r(7, 8, 1e3)  # C9,R13
    adm(8, -1, 2 * 2e-7 / 0.041)  # Differential admittance of D2,D3
    c(8, 9, 1e-6); r(9, 15, 47e3)  # C10,R16
    c(9, 10, 2.2e-9); r(10, 13, 22e3)  # C11,R15
    r(0, 11, 5.1e3); c(0, 11, 68e-9)  # R5,C4
    r(11, -1, 1.5e3); c(11, 12, 390e-9); r(12, -1, 1e3)  # R8,C6,R9
    r(11, 13, gain * 100e3); r(13, -1, (1-gain) * 100e3)  # P1A
    r(13, 15, 27e3); r(13, 14, 12e3); c(14, 15, 27e-9)  # R17,R18,C12
    r(15, 16, 392e3); c(15, 16, 820e-12)  # R20,C13
    r(16, 17, 100e3); r(17, 19, 100e3)  # R22,R241
    r(16, 18, 1.8e3 + (1-treble)*10e3)  # R21 + P2 upper
    r(18, 19, 4.7e3 + treble*10e3); c(18, 17, 3.9e-9)  # R23 + P2 lower,C14
    c(0, 20, 4.7e-6); r(20, -1, 100e3); r(20, 22, 68_560)  # C2,R3,R4+R26
    c(19, 21, 4.7e-6); r(21, 22, 560 + (1-output)*10e3)  # C15,R25+P3 upper
    r(22, -1, output*10e3); r(22, -1, 100e3)  # P3 lower,R28
    return a, b, capacitors


def reference(rate, hz, gain, treble, output):
    s = 2j * rate * math.tan(math.pi * hz / rate)
    a, b, _ = network(s, gain, treble, output)
    return solve(a, b)[:23]


if __name__ == '__main__':
    rows = ['# rate,hz,gain,treble,output, then real/imaginary pairs for nodes 0..22']
    for i, gain in enumerate([0.0, 0.25, 0.5, 1.0]):
        for j, treble in enumerate([0.0, 0.5, 1.0]):
            rate = [352800, 384000, 768000][j]
            hz = [80, 1000, 8000][(i+j) % 3]
            output = [0.25, 0.5, 1.0][(i+2*j) % 3]
            nodes = reference(rate, hz, gain, treble, output)
            row = [rate, hz, gain, treble, output]
            row.extend(v for node in nodes for v in [node.real, node.imag])
            rows.append(','.join(format(v, '.17g') for v in row))
    Path(__file__).with_name('ac.csv').write_text('\n'.join(rows) + '\n')
