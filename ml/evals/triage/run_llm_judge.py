#!/usr/bin/env python3
"""CrisisGraph LLM-as-a-Judge Multilingual Evaluation & Self-Refinement Benchmark.

Zero-dependency standard library implementation (runs on standard python3).
Evaluates extraction accuracy, translation fidelity, and topological graph snapability.
"""

import json
import os
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any, Dict, List, Optional

ML_ROOT = Path(__file__).resolve().parents[2]
DATASET_PATH = ML_ROOT / "data" / "multilingual-triage-eval.jsonl"
SYSTEM_PROMPT_PATH = ML_ROOT / "prompts" / "triage-extraction-v2.1.1.txt"
SCHEMA_PATH = ML_ROOT.parent / "schemas" / "triage-extraction-v2.json"
JUDGE_PROMPT_PATH = ML_ROOT / "prompts" / "triage-judge-v1.txt"
REPORT_OUTPUT_PATH = ML_ROOT / "reports" / "llm_judge_report.json"

BASE_URL = os.environ.get("INFERENCE_BASE_URL", "http://localhost:8000/v1")
MODEL_NAME = os.environ.get("INFERENCE_MODEL", "Qwen/Qwen3-4B-Instruct-2507")

# Deterministic Spatial Resolver Landmarks (mirrors Rust SpatialResolver)
PILOT_LANDMARKS = {
    "aluva manappuram": "node/7048449098",
    "manappuram": "node/7048449098",
    "shiva temple aluva": "node/7048449098",
    "manappuram temple": "node/7048449098",
    "aluva railway station": "node/4664235699",
    "aluva station": "node/4664235699",
    "railway station": "node/4664235699",
    "aluva town hall": "node/4664235729",
    "town hall aluva": "node/4664235729",
    "civil station": "node/4664235729",
    "uc college": "node/9903123560",
    "uc college aluva": "node/9903123560",
    "union christian college": "node/9903123560",
    "aluva taluk hospital": "node/343716109",
    "taluk hospital": "node/343716109",
    "govt hospital aluva": "node/343716109",
    "pump junction": "node/7992454789",
    "pump jn": "node/7992454789",
    "bank junction": "node/4742583964",
    "bank jn": "node/4742583964",
}


def resolve_landmark(location_str: Optional[str]) -> Optional[str]:
    if not location_str:
        return None
    cleaned = location_str.strip().lower()
    for landmark, junction in sorted(PILOT_LANDMARKS.items(), key=lambda x: len(x[0]), reverse=True):
        if landmark in cleaned or cleaned in landmark:
            return junction
    return None


def query_llm(messages: List[Dict[str, str]], temperature: float = 0.0, response_format: Optional[dict] = None) -> str:
    payload = {
        "model": MODEL_NAME,
        "messages": messages,
        "temperature": temperature,
    }
    if response_format:
        payload["response_format"] = response_format

    req = urllib.request.Request(
        f"{BASE_URL}/chat/completions",
        data=json.dumps(payload).encode("utf-8"),
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=60) as resp:
        data = json.loads(resp.read().decode("utf-8"))
        return data["choices"][0]["message"]["content"]


def extract_triage(report_text: str, system_prompt: str, schema: dict) -> dict:
    resp_format = {
        "type": "json_schema",
        "json_schema": {
            "name": "crisisgraph_triage_extraction",
            "strict": True,
            "schema": schema,
        },
    }
    content = query_llm(
        [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": report_text},
        ],
        temperature=0.0,
        response_format=resp_format,
    )
    return json.loads(content)


def refine_landmark_loop(report_text: str, candidate_loc: str) -> Optional[str]:
    system_msg = (
        "You are a geospatial landmark normalizer for Aluva emergency dispatch. "
        "Map the reported location to one of these exact landmark names: "
        "['Aluva Manappuram', 'Aluva Railway Station', 'Aluva Town Hall', 'UC College', 'Aluva Taluk Hospital', 'Pump Junction', 'Bank Junction']. "
        "If it refers to one of them, return ONLY that landmark name. If unknown or not matching, return 'NONE'."
    )
    user_msg = f'Report: "{report_text}"\nExtracted location: "{candidate_loc}"\nMatching landmark:'
    ans = query_llm(
        [
            {"role": "system", "content": system_msg},
            {"role": "user", "content": user_msg},
        ],
        temperature=0.0,
    ).strip().strip('"')
    if ans != "NONE" and ans:
        return resolve_landmark(ans)
    return None


def judge_extraction(
    judge_prompt: str,
    raw_input: str,
    extracted_json: dict,
    resolved_junction: Optional[str],
    expected: dict,
) -> dict:
    prompt_content = (
        f'CITIZEN_REPORT:\n"{raw_input}"\n\n'
        f"EXTRACTED_JSON:\n{json.dumps(extracted_json, indent=2)}\n\n"
        f"RESOLVED_JUNCTION:\n{resolved_junction}\n\n"
        f"EXPECTED_FACTS:\n{json.dumps(expected, indent=2)}"
    )

    raw = query_llm(
        [
            {"role": "system", "content": judge_prompt},
            {"role": "user", "content": prompt_content},
        ],
        temperature=0.0,
    )
    cleaned = raw.strip()
    if cleaned.startswith("```"):
        cleaned = cleaned.split("```")[1]
        if cleaned.startswith("json"):
            cleaned = cleaned[4:]
        cleaned = cleaned.strip()
    return json.loads(cleaned)


def main():
    print("=" * 80)
    print("CrisisGraph: Multilingual Triage Extraction & LLM-as-a-Judge Evaluation")
    print(f"Inference Endpoint: {BASE_URL} (Model: {MODEL_NAME})")
    print(f"Dataset: {DATASET_PATH}")
    print("=" * 80)

    if not DATASET_PATH.exists():
        print(f"Error: dataset {DATASET_PATH} not found", file=sys.stderr)
        sys.exit(1)

    system_prompt = SYSTEM_PROMPT_PATH.read_text(encoding="utf-8").strip()
    schema = json.loads(SCHEMA_PATH.read_text(encoding="utf-8"))
    judge_prompt = JUDGE_PROMPT_PATH.read_text(encoding="utf-8").strip()

    cases = [json.loads(line) for line in DATASET_PATH.read_text(encoding="utf-8").splitlines() if line.strip()]
    print(f"Loaded {len(cases)} multilingual benchmark cases.\n")

    results = []
    total_score = 0
    passed_count = 0

    for idx, case in enumerate(cases, 1):
        case_id = case["id"]
        raw_text = case["input"]
        expected = case["expected"]
        lang = case.get("language", "unknown")

        start_t = time.perf_counter()
        # 1. Primary extraction (Single-Pass with translation)
        try:
            extraction = extract_triage(raw_text, system_prompt, schema)
        except Exception as e:
            print(f"[{idx:02d}/{len(cases):02d}] {case_id:<32} Extraction FAILED - {e}")
            continue

        # 2. Spatial resolution
        victim_loc = extraction.get("victim_location")
        resolved_junction = resolve_landmark(victim_loc)

        # In-Engine Refinement Loop if unmapped
        refined = False
        if not resolved_junction and victim_loc:
            refinement = refine_landmark_loop(raw_text, victim_loc)
            if refinement:
                resolved_junction = refinement
                refined = True

        # 3. LLM-as-a-Judge Evaluation
        try:
            verdict = judge_extraction(judge_prompt, raw_text, extraction, resolved_junction, expected)
        except Exception as e:
            print(f"[{idx:02d}/{len(cases):02d}] {case_id:<32} Judge Evaluation FAILED - {e}")
            verdict = {"total_score": 0, "verdict": "ERROR", "critique": str(e)}

        elapsed = (time.perf_counter() - start_t) * 1000.0
        score = verdict.get("total_score", 0)
        status = verdict.get("verdict", "FAIL")

        total_score += score
        if status == "PASS":
            passed_count += 1

        results.append({
            "case_id": case_id,
            "language": lang,
            "input": raw_text,
            "extraction": extraction,
            "resolved_junction": resolved_junction,
            "refinement_triggered": refined,
            "expected": expected,
            "judge_verdict": verdict,
            "latency_ms": elapsed,
        })

        tag = "PASS" if status == "PASS" else ("BORDERLINE" if status == "BORDERLINE" else "FAIL")
        snap_tag = f"-> {resolved_junction}" if resolved_junction else "(Unmapped)"
        refine_tag = " [Refined in Loop]" if refined else ""
        print(f"[{idx:02d}/{len(cases):02d}] {case_id:<32} {tag:<10} Score: {score:3d}/100  Loc: '{victim_loc}' {snap_tag}{refine_tag} ({elapsed:.0f}ms)")

    # Print Summary Report
    avg_score = total_score / len(results) if results else 0
    pass_rate = (passed_count / len(results) * 100.0) if results else 0

    print("\n" + "=" * 80)
    print("EVALUATION BENCHMARK SUMMARY")
    print("=" * 80)
    print(f"Total Test Cases:       {len(results)}")
    print(f"Pass Rate:              {pass_rate:.1f}% ({passed_count}/{len(results)})")
    print(f"Average Judge Score:    {avg_score:.1f} / 100")
    print("=" * 80)

    REPORT_OUTPUT_PATH.parent.mkdir(parents=True, exist_ok=True)
    REPORT_OUTPUT_PATH.write_text(
        json.dumps(
            {
                "model": MODEL_NAME,
                "pass_rate_pct": pass_rate,
                "average_score": avg_score,
                "cases_evaluated": len(results),
                "results": results,
            },
            indent=2,
            ensure_ascii=False,
        )
        + "\n",
        encoding="utf-8",
    )
    print(f"Detailed evaluation report saved to: {REPORT_OUTPUT_PATH}\n")


if __name__ == "__main__":
    main()
