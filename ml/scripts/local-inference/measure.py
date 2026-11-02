"""Measure where local extraction time goes (prefill vs decode, cache reuse).

Sends exactly what the Rust extractor sends (system prompt v3.2, schema v3 as
json_schema, temperature 0, max_tokens 768) and, between extractions, the
landmark-refinement call with its different system prompt, as the pipeline
does. Prints llama.cpp's own per-request timings.

usage: python3 -I ml/scripts/local-inference/measure.py <repo_root> <base_url> <sequence>
  sequence: comma list of E (extraction) and R (refinement), e.g. E,E,R,E
"""
import json
import sys
import time
import urllib.request

root, base, seq = sys.argv[1], sys.argv[2], sys.argv[3].split(",")
system = open(f"{root}/ml/prompts/triage-extraction-v3.2.txt").read()
schema = json.load(open(f"{root}/schemas/triage-extraction-v3.json"))
REFINE_SYSTEM = (
    "You are a geospatial landmark normalizer for Aluva emergency dispatch. "
    "Map the reported location to one of these exact landmark names: "
    "['Aluva Manappuram', 'Aluva Railway Station', 'Aluva Town Hall', 'UC College', 'Aluva Taluk Hospital', "
    "'Pump Junction', 'Bank Junction']. If the report refers to one of them, return ONLY that landmark name. "
    "If unknown or unrelated, return 'NONE'."
)
# Synthetic messages (no evaluation-set content).
messages = [
    "Water is waist deep inside our house near Aluva town hall and rising. 3 of us, my grandmother cannot walk. I am here with them. Please send an ambulance.",
    "Achan bodham illa, vellam nenju vare und, koodunnu. 4 aalukal und. Njan ivide und. Ambulance ayakkoo.",
    "Roadil kaal vare vellam, 5 peru camp-ilekk pokanam. Njan ivide und. Truck ayakkoo.",
    "Please help, we are 2 people near the temple, the situation is bad. I am here with them. Please send an ambulance.",
    "വീട്ടിൽ അരയോളം വെള്ളം, അമ്മൂമ്മയും ഉണ്ട്, ഞങ്ങൾ 3 പേർ. ഞാൻ ഇവിടെയുണ്ട്. ആംബുലൻസ് അയക്കൂ.",
]


def post(payload):
    req = urllib.request.Request(f"{base}/chat/completions", data=json.dumps(payload).encode(),
                                 headers={"content-type": "application/json"})
    t = time.time()
    with urllib.request.urlopen(req, timeout=600) as r:
        body = json.load(r)
    return body, time.time() - t


print("step kind  wall_s  cached  new_prompt  prompt_s  prompt_tok/s  gen_tok  gen_s  gen_tok/s")
n = 0
for i, kind in enumerate(seq):
    msg = messages[n % len(messages)]
    if kind == "E":
        n += 1
        payload = {"model": "local", "temperature": 0.0, "max_tokens": 768,
                   "messages": [{"role": "system", "content": system}, {"role": "user", "content": msg}],
                   "response_format": {"type": "json_schema", "json_schema": {
                       "name": "crisisgraph_triage_extraction", "strict": True, "schema": schema}}}
    else:
        payload = {"model": "local", "temperature": 0.0, "max_tokens": 25,
                   "messages": [{"role": "system", "content": REFINE_SYSTEM},
                                {"role": "user", "content": f'Report: "{msg}"\nExtracted location: "near the house"\nMatching landmark:'}]}
    body, wall = post(payload)
    t = body.get("timings", {})
    print(f"{i+1:>4} {kind:>4}  {wall:6.1f}  {t.get('cache_n', '-'):>6}  {t.get('prompt_n', '-'):>10}  "
          f"{t.get('prompt_ms', 0)/1000:8.1f}  {t.get('prompt_per_second', 0):12.1f}  {t.get('predicted_n', '-'):>7}  "
          f"{t.get('predicted_ms', 0)/1000:5.1f}  {t.get('predicted_per_second', 0):9.1f}", flush=True)
