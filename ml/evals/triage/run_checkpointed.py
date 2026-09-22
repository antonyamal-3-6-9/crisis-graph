"""Run the existing evaluator serially, preserving each completed response."""

import asyncio
import hashlib
import json
from dataclasses import asdict

import httpx
from jsonschema import Draft202012Validator

from crisisgraph_ml.triage_eval import (
    CaseResult,
    evaluate_case,
    load_cases,
    load_json,
    parse_args,
    summarize,
)


async def main():
    args = parse_args()
    if args.concurrency != 1 or args.output is None:
        raise ValueError("Checkpointed runner requires concurrency 1 and an output path")
    if args.output.exists():
        raise FileExistsError(f"Completed output already exists: {args.output}")
    cases = load_cases(args.dataset)
    prompt = args.prompt.read_text(encoding="utf-8").strip()
    schema = load_json(args.schema)
    validator = Draft202012Validator(schema)
    checkpoint = args.output.with_suffix(".checkpoint.jsonl")
    identity_path = args.output.with_suffix(".identity.json")
    identity = {
        "model": args.model,
        "backend": args.backend,
        "base_url": args.base_url,
        "dataset_sha256": hashlib.sha256(args.dataset.read_bytes()).hexdigest(),
        "prompt_sha256": hashlib.sha256(args.prompt.read_bytes()).hexdigest(),
        "schema_sha256": hashlib.sha256(args.schema.read_bytes()).hexdigest(),
    }
    # Output filename/run identity must refer to a fixed adapter and serving configuration.
    if identity_path.exists():
        if load_json(identity_path) != identity:
            raise ValueError("Checkpoint input identity changed")
    else:
        if checkpoint.exists():
            raise ValueError("Checkpoint has no input identity")
        identity_path.parent.mkdir(parents=True, exist_ok=True)
        identity_path.write_text(json.dumps(identity, indent=2) + "\n", encoding="utf-8")
    results = []
    if checkpoint.exists():
        results = [CaseResult(**json.loads(line)) for line in checkpoint.read_text().splitlines()]
        if [r.case_id for r in results] != [c["id"] for c in cases[: len(results)]]:
            raise ValueError("Checkpoint order differs from dataset")
    semaphore = asyncio.Semaphore(1)
    async with httpx.AsyncClient(timeout=httpx.Timeout(args.timeout)) as client:
        for case in cases[len(results) :]:
            result = await evaluate_case(
                client,
                semaphore,
                case,
                args.backend,
                args.base_url,
                args.model,
                prompt,
                schema,
                validator,
            )
            with checkpoint.open("a", encoding="utf-8") as handle:
                handle.write(json.dumps(asdict(result), ensure_ascii=False) + "\n")
                handle.flush()
            results.append(result)
            print(f"{len(results)}/{len(cases)} {result.case_id} error={result.error}", flush=True)
    report = summarize(results, args.model, args.backend)
    args.output.write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    print(
        json.dumps(
            {k: v for k, v in report.items() if k not in {"results", "by_category", "by_language"}},
            indent=2,
        )
    )


if __name__ == "__main__":
    asyncio.run(main())
