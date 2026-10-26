"""Markdown review sheets for V3 labelled cases (owner review without editing JSON)."""

from __future__ import annotations

import argparse
from pathlib import Path
from typing import Any

from crisisgraph_ml.triage_eval import load_cases
from crisisgraph_ml.urgency import RULES_VERSION, derive_urgency

ABBREVIATIONS = {
    "reporter_relation": "rel",
    "reporter_on_scene": "on_scene",
    "medical_conditions": "med",
    "other_conditions": "other",
    "vulnerable_groups": "groups",
    "water_level": "water",
    "water_trend": "trend",
}


def profile_cell(profile: dict[str, Any]) -> str:
    parts = [
        f"{ABBREVIATIONS[key]}: {', '.join(value) if isinstance(value, list) else value}"
        for key, value in profile.items()
        if key in ABBREVIATIONS and value not in (None, [])
    ]
    return "<br>".join(parts) or "—"


def v2_cell(expected: dict[str, Any]) -> str:
    cell = (
        f"loc: {expected['victim_location']}<br>count: {expected['headcount']}"
        f"<br>asset: {expected['required_asset']}"
    )
    cell += "<br>hazards: " + ("; ".join(
        f"{h['road_segment']} {h['status']} {h['duration_hours']}" for h in expected["hazards"]
    ) or "none")
    return cell + f"<br>review: {expected['needs_human_review']}"


def render(cases: list[dict[str, Any]], title: str, note: str, source: str,
           rules: str = RULES_VERSION) -> str:
    lines = [
        f"# {title}",
        "",
        "Review each case against `triage-annotation-guidelines.md` and",
        "`triage-annotation-guidelines-v3.1-changes.md`. Mark **OK**, or write the",
        "correction in the last column (e.g. `water: CHEST`, `groups: +ELDERLY`,",
        f"`gold: URGENT`). Corrections are applied to `{source}`; do not edit the JSONL",
        "by hand.",
        "",
        "- **Profile** lists only non-empty fields; anything missing is null / [].",
        f"- **Gold** is the annotator's clinical-sense urgency; **Rule** is what {rules}",
        "  gives on these labels. ⚠ marks a mismatch (rule lower than gold is a",
        "  potential under-triage; rule higher is over-triage).",
        "- **Note** is the annotator's own comment, if any.",
        "",
        note,
        "",
    ]
    category = None
    for case in cases:
        if case["category"] != category:
            category = case["category"]
            lines += ["", f"## {category}", "",
                      "| ID | Lang | Message | V2 labels | Profile | Gold / Rule | Note | Review |",
                      "|---|---|---|---|---|---|---|---|"]
        expected = case["expected"]
        result = derive_urgency(expected["victim_profile"], rules)
        rule = result.rule_urgency + (
            f" ({result.review_priority})" if result.review_priority else ""
        )
        gold = case["expected_urgency"]
        mark = " ⚠" if gold != result.rule_urgency else ""
        message = case["input"].replace("|", "\\|")
        note_text = case.get("annotator_note", "").replace("|", "\\|")
        lines.append(
            f"| {case['id'].split('-')[-1]} | {case['language']} | {message} | "
            f"{v2_cell(expected)} | {profile_cell(expected['victim_profile'])} | "
            f"{gold} / {rule}{mark} | {note_text} |  |"
        )
    return "\n".join(lines) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cases", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--title", required=True)
    parser.add_argument("--note", default="")
    parser.add_argument("--source", default="the candidates file")
    args = parser.parse_args()
    cases = load_cases(args.cases)
    args.output.write_text(render(cases, args.title, args.note, args.source), encoding="utf-8")
    print(f"Wrote review sheet for {len(cases)} cases to {args.output}")


if __name__ == "__main__":
    main()
