"""Turn llama-bench CSV (stdin) into lines: test<TAB>avg_tps<TAB>sd_tps."""
import csv
import sys

for row in csv.DictReader(sys.stdin):
    test = ("pp" + row["n_prompt"]) if row["n_prompt"] != "0" else ("tg" + row["n_gen"])
    print(f"{test}\t{row['avg_ts']}\t{row['stddev_ts']}")
