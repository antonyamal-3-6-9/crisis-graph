# Qwen3-4B SFT V4 training receipt

Imported the user-provided `qwen3-4b-qlora-sft-v4-full-adapter.zip` into ignored
`ml/artifacts/qwen3-4b-triage-sft-v4/`; extracted contents are under `adapter/`.
The original Downloads file was not modified. ZIP integrity checks passed.

ZIP SHA-256: `c7ae0649723e524267523d0af3df4245aa394d23dbf51b6a4772a67c16d38813`.

The bundled manifest records a full two-epoch QLoRA run on a Tesla T4 (Kaggle,
commit mode, not resumed) using the pinned Qwen3-4B-Instruct-2507 base revision
`cdbee75f17c01a7cc42f958dc650907174af0554`, trained from the untouched base.
It used 608 training and 102 validation records with V3 targets and prompt
v3.2 as the system message. The recorded dataset hashes exactly match the
exports in `data/triage-sft-v4-export-manifest.json`:

- Train: `64a0a46d6a9f62290c08e1e636ea95503fb5e59952325cdc27b216488847be29`
- Validation: `72d7b3ef9b0c167af5ae944e718c0e64d45853b3e1cb08aeb49bf81e1bf087ea`

The adapter configuration records QLoRA/NF4, rank 16, alpha 32, dropout 0.05,
and all linear modules as targets, as for SFT-V3. Training used seed 42,
maximum length 3,072 (longest tokenised record 2,847, so nothing was
truncated), micro-batch size one, gradient accumulation eight, learning rate
`1e-4`, six warmup steps and assistant-only loss. The only planned change from
SFT-V3 is the maximum length.

| Training observation | Value |
|---|---:|
| Runtime | 10,100.8 seconds (2.81 hours) |
| Mean training loss | 0.1332 |
| Validation loss (selected checkpoint) | 0.0318 |
| Teacher-forced validation token accuracy | 0.9945 |
| Selected checkpoint | 76 (end of epoch 1) |

The notebook selects the epoch-end checkpoint with the lower validation loss;
epoch 1 was lower than epoch 2, a mild sign of overfitting in the second epoch.
The per-epoch losses themselves were not saved in the manifest. These values
come from the saved manifest; training was not repeated locally.
Teacher-forced token accuracy is not extraction accuracy or a generalisation
measurement: 72 of the 102 validation records are converted SFT-V3 records with
empty profiles.

All training labels are owner-reviewed. New message texts were written by
zero-context agents that never saw the evaluation sets
(`data/triage-sft-v4-spec.md`, revision 2026-10-28;
`data/triage-sft-v4-leak-audit.json`).

The PEFT adapter was converted locally to a separate 504-tensor F16 GGUF for
llama.cpp (`convert_lora_to_gguf.py` from llama.cpp `5266f24da7`, transformers
4.57.6, torch 2.11.0, `--outtype f16`). Its SHA-256 is
`f31100c925b3b2f4540f481be0f9338d5024ca931c0368265f6b16eb5f520fbc`.
The original PEFT files remain unchanged. Evaluation results are reported
separately.
