"""Full-circuit transient reference, independent of the Rust stage solver.

Uses the AC reference netlist, trapezoidal capacitors, rail-limited U1B with
dominant-pole bandwidth and bisection for the diode pair. No FIRs: tests the internal analog circuit.
Regenerate with Python 3; no external packages required.
"""
import math
from pathlib import Path
from ac_reference import network, solve


def inverse(a):
    cols = []
    for i in range(len(a)):
        b = [0j] * len(a)
        b[i] = 1
        cols.append(solve([row[:] for row in a], b))
    return list(zip(*cols))


def run(gain):
    rate, treble, output = 384000, 0.5, 0.7
    a, _, caps = network(2*rate, gain, treble, output)
    matrices = [inverse(a)]
    pinned = [row[:] for row in a]
    pinned[24] = [0j] * len(a)
    pinned[24][5] = 1
    matrices.append(inverse(pinned))
    histories = [0.0] * len(caps)
    input_history = 0.0
    previous_error = previous_output = 0.0
    g_input = 2 * rate * 100e-9
    values = []
    saturations = 0
    for sample in range(4096):
        t = sample / rate
        raw = 0.7*math.sin(2*math.pi*700*t) + 0.1*math.sin(2*math.pi*3100*t)
        current = (g_input*raw + input_history) / (1 + g_input*1_010_000)
        input_history = -g_input * (raw - current*1_010_000) - current
        rhs = [0.0]*len(a)
        rhs[23] = current * 1_000_000
        rhs[24] = -previous_error - previous_output*(2*rate-2*math.pi*3e6/200e3)/(2*math.pi*3e6)
        for (p, q, _), history in zip(caps, histories):
            if p >= 0:
                rhs[p] -= history
            if q >= 0:
                rhs[q] += history
        inv = matrices[0]
        nodes = [sum(y.real*x for x, y in zip(rhs, row)) for row in inv]
        if abs(nodes[5]) > 3:
            rhs[24] = math.copysign(3, nodes[5])
            inv = matrices[1]
            nodes = [sum(y.real*x for x, y in zip(rhs, row)) for row in inv]
            saturations += 1
        # Diodes are already linearized in the AC netlist. Remove that
        # differential conductance with a rank-one inverse correction first.
        gd = 2*2e-7/0.041
        correction = 1 - gd*inv[8][8].real
        col = [row[8].real/correction for row in inv]
        open_voltage = nodes[8]/correction
        nodes = [v + col[i]*gd*nodes[8] for i, v in enumerate(nodes)]
        lo, hi = 0, abs(open_voltage)
        z = col[8]
        for _ in range(64):
            v = (lo+hi)/2
            # The upper bracket may initially exceed exp()'s range.
            f = float('inf') if v/0.041 > 700 else v + z*4e-7*math.sinh(v/0.041)
            if f > abs(open_voltage):
                hi = v
            else:
                lo = v
        diode = math.copysign((lo+hi)/2, open_voltage)
        diode_current = (open_voltage-diode)/z
        nodes = [v-col[i]*diode_current for i,v in enumerate(nodes)]
        for i, (p,q,c) in enumerate(caps):
            v = (nodes[p] if p >= 0 else 0) - (nodes[q] if q >= 0 else 0)
            histories[i] = -4*rate*c*v - histories[i]
        previous_error = nodes[2]-nodes[3]
        previous_output = nodes[5]
        assert abs(nodes[16]) < 11.5 and abs(nodes[19]) < 11.5
        if sample % 32 == 31:
            values.append(nodes[22])
    return [rate, gain, treble, output, saturations, *values]


if __name__ == '__main__':
    rows = ['# rate,gain,treble,output,saturated-U1B-samples, output after every 32 samples']
    for gain in [0.0, 0.5, 1.0]:
        rows.append(','.join(format(v, '.17g') for v in run(gain)))
    Path(__file__).with_name('transient.csv').write_text('\n'.join(rows)+'\n')
