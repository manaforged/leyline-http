import collections
import math
import os
import statistics
import sys

T975 = [12.706, 4.303, 3.182, 2.776, 2.571, 2.447, 2.365, 2.306, 2.262, 2.228]
MIN_ROUNDS = int(os.environ.get("STOP_MIN_ROUNDS", "3"))
MARGIN = math.log(1 + float(os.environ.get("STOP_MARGIN", "0.05")))


def decided(logs):
    n = len(logs)
    if n < MIN_ROUNDS:
        return False
    half = T975[min(n, len(T975)) - 2] * statistics.stdev(logs) / math.sqrt(n)
    mean = statistics.mean(logs)
    return mean - half > MARGIN or mean + half < -MARGIN


def main(path, ref):
    runs = collections.defaultdict(dict)
    for line in open(path):
        p = line.split()
        if len(p) != 11:
            continue
        added = int(p[7]) - int(p[3])
        rate = added * 1e9 / (int(p[10]) - int(p[6]))
        cpu = (float(p[9]) - float(p[5])) / added
        runs[(p[1], p[2])][p[0]] = (rate, cpu)
    for (cell, lib), rounds in runs.items():
        base = runs.get((cell, ref))
        if lib == ref or not base:
            continue
        shared = [r for r in rounds if r in base]
        rate = [math.log(base[r][0] / rounds[r][0]) for r in shared]
        cpu = [math.log(base[r][1] / rounds[r][1]) for r in shared]
        if decided(rate) and decided(cpu):
            print(cell, lib)


main(sys.argv[1], sys.argv[2])
