# Qwen3-4B SFT V3 training receipt

Imported the user-provided `qwen3-4b-qlora-sft-v3-full-adapter.zip` into ignored
`ml/artifacts/qwen3-4b-triage-sft-v3/`; extracted contents are under `adapter/`.
The original Downloads file was not modified. ZIP integrity checks passed.

ZIP SHA-256: `045fe83212bba364e30695973311976b416237713fb91147213bebefe47fb939`.

The bundled manifest records a full two-epoch QLoRA run on a Tesla T4 using the
pinned Qwen3-4B-Instruct-2507 base revision. It used 300 training and 72
validation examples. The recorded dataset hashes exactly match the locally
generated V3 exports:

- Train: `4c6ffd57242df8e7d076914ab14015c0d8691db2a4783750c6a9152dfaf30db9`
- Validation: `d22a5d10e02bda2297fd3d07b98ca7b5b3842dc7c989987dd9f3a731ef696351`

The adapter configuration records QLoRA/NF4, rank 16, alpha 32, dropout 0.05,
and all linear modules as targets. Training used seed 42, maximum length 1,024,
micro-batch size one, gradient accumulation eight, learning rate `1e-4`, six
warmup steps, assistant-only loss, and checkpoint 76.

| Training observation | Value |
|---|---:|
| Runtime | 1,566.2793 seconds (26.10 minutes) |
| Mean training loss | 0.26530472897789686 |
| Validation loss | 0.010270615108311176 |
| Teacher-forced validation token accuracy | 0.9968535030881563 |

These values come from the saved manifest; local GPU training was not repeated.
Teacher-forced token accuracy is not extraction accuracy, review recall, or a
generalization measurement. The manifest also says independent review was not
complete, so this artifact must not be described as independently reviewed.

The PEFT adapter was converted locally to a separate 504-tensor F16 GGUF for
llama.cpp. Its SHA-256 is
`bcec4e21b232174b89dcd0dd2a800212136488ed32d962e810babae9efa18039`.
The original PEFT files remain unchanged. Frozen regression results and the
promotion decision are documented in `qwen3-4b-qlora-sft-v3-regression.md`.
