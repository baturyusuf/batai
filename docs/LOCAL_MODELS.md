# Local Models and Batai Benchmark

## Hardware profile

`HardwareProfiler` uses a cross-platform system inventory for CPU, physical/logical cores, total/current RAM, OS and available disk. NVIDIA details come from one structured `nvidia-smi` invocation with fixed arguments for model, total/free VRAM and driver version. Missing executables, malformed output, AMD/Apple devices and unsupported GPUs degrade to an unknown GPU instead of crashing.

The hardware fingerprint includes stable CPU, total RAM, GPU/VRAM/driver and OS identity. It deliberately excludes current free RAM and VRAM. Benchmark results are keyed by model, suite version and this fingerprint so a significant hardware change invalidates their direct applicability.

## Ollama lifecycle

The local adapter distinguishes `NOT_INSTALLED`, `INSTALLED_OFFLINE`, `RUNNING` and `ERROR`. It uses the official local endpoints for installed models, model details, running models, streaming pull progress, explicit cancellation, deletion and inference. Downloads start only from an explicit user action and only after a free-disk safety check. Progress is derived from Ollama's stream; no synthetic percentage is shown. After restart the UI refreshes installed/running truth from Ollama.

The curated v1 catalog is intentionally small. Disk, VRAM and context values are conservative planning estimates, labeled as estimates and never reused as capability scores. Hardware fit is `EXCELLENT`, `GOOD`, `PARTIAL_OFFLOAD`, `POOR` or `UNSUPPORTED`. A configurable VRAM margin (20% in the current UI snapshot) reserves capacity for the OS, display and runtime. CPU/RAM partial offload is called out as potentially slower.

## Local benchmark

The optional suite uses isolated synthetic prompts; it never uploads or reads repository source. Categories are coding, debugging, script transformation, test generation, planning, strict instruction following and tool/patch manifest generation. Temperature is zero where Ollama supports it.

Scoring is deterministic: expected fixture tokens, JSON validity and expected-output checks produce correctness; retries reduce score. The runtime records latency, prompt/evaluation timing when reported, token throughput, token counts and failure state. Peak memory remains unknown unless a reliable runtime measurement exists.

Dimension scores retain `BATAI_BENCHMARK` evidence, timestamp, suite version, sample count and hardware fingerprint. Unknown architecture, long-context and research scores stay `null` when this short suite does not measure them.

Validated Batai task outcomes are stored separately as `REAL_TASK_HISTORY` rather than rewriting these benchmark observations. Quality evidence may aggregate for the exact model identity, while latency evidence for local execution is filtered by hardware fingerprint. The Resources capability drawer shows both sources, confidence and disagreement. See [Capability Learning](CAPABILITY_LEARNING.md).

Role recommendations reuse the organization function policy. The same model can be a Junior Software Engineer and a Senior Product Analyst. Automatic recommendations stop at L4 Senior; L5+ remains a governance-mediated suggestion.

## Safety and limitations

- Model download is never automatic, including during agent creation.
- Hardware fit is an estimate; benchmark evidence is the operational signal.
- Local monetary API cost is not the same as energy cost. Energy and depreciation are not estimated.
- The benchmark is a worker-fit check, not a general leaderboard or model certification.
- Ollama cancellation stops Batai's HTTP stream; the daemon's own layer download behavior remains governed by its official API semantics.
