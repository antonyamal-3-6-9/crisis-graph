"""Print llama-bench generation results by context depth.

usage: python3 -I parse_depth.py <bench.csv> <label>
"""
import csv
import sys

path, label = sys.argv[1], sys.argv[2]
for row in csv.DictReader(open(path)):
    print(f"{label:<22} depth {row['n_depth']:>5}  tg{row['n_gen']} "
          f"{float(row['avg_ts']):6.2f} +- {float(row['stddev_ts']):.2f} tokens/s")
