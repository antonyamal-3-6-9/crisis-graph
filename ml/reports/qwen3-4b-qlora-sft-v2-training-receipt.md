# Qwen3-4B SFT V2 training receipt

Imported the user-provided `qwen3-4b-qlora-sft-v2-full-adapter.zip` into ignored
`ml/artifacts/qwen3-4b-triage-sft-v2/`; extracted contents are under `adapter/`.
The original Downloads file is unchanged. ZIP integrity checks passed.

ZIP SHA-256: `cd450b5b89bac352863bce6cad908a15f52afdd1ffd905d2a3a5e53a6fb5377b`.

The bundled manifest records a full two-epoch QLoRA run on a Tesla T4, with 240
training and 48 validation examples. Both data hashes match the V2 export manifest.
The base revision and all seven recorded package versions match the saved V1 run.
Adapter config confirms rank 16, alpha 32, dropout 0.05 and the seven expected
linear-module targets. The run selected checkpoint-60.

| Training observation | Value |
|---|---:|
| Runtime | 1342.5577 seconds (22.38 minutes) |
| Mean training loss | 0.36665444768344363 |
| Validation loss | 0.031673938035964966 |
| Teacher-forced validation token accuracy | 0.9937744277218977 |

These observations come from the saved run manifest; local GPU training was not
repeated. V1 and V2 use different validation examples, so their validation losses
are not a direct model ranking. Token accuracy is not free-generation extraction
accuracy or human-review recall. At receipt time the adapter had not been evaluated.
The local regression evaluation is now complete; see
`qwen3-4b-qlora-sft-v2-regression.md`. V2 was not promoted. The evaluation procedure
was to convert a separate adapter GGUF with the same llama.cpp converter
used for V1 and run the frozen 120-case regression suite with prompt V2.1.1 and
matching serving settings. Compare base, V1, and V2; a new blind holdout remains
necessary for an unbiased final generalization claim.
