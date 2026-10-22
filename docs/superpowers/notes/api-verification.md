# API verification (spec section 11, items 2 to 4)

Verified 2026-10-22 against Ollama 0.40.8 (macOS, `runner` values seen: `ggml`, `llamacpp`, `mlx`) and the public Hugging Face API without a token. Fixtures are in `crates/hf2ollama-core/tests/fixtures/`. The real import checks used only the temporary model `hf2ollama-verify-tmp`, deleted afterwards (`/api/tags` no longer lists it, `/api/show` returns 404).

Test repos: GGUF `ggml-org/tiny-llamas` (`stories260K.gguf`, 1.2 MB); safetensors `hf-internal-testing/tiny-random-LlamaForCausalLM` (4 MB weights, 7 files); gated `meta-llama/Llama-3.2-1B`.

## (a) Per-file SHA-256 on Hugging Face

`GET /api/models/<repo>?blobs=true` returns `siblings[]` with `rfilename`, `blobId` (git SHA-1), `size`, and for LFS files `lfs: { sha256, size, pointerSize }`. Example: `stories260K.gguf` has `lfs.sha256 = 047bf464...c04b`; the downloaded file hashed to the same value. Without `blobs=true` siblings only carry `rfilename`.

Caveat: `lfs` exists only for LFS files. Small non-LFS files (`config.json`, `tokenizer.json` at 1.8 MB, `tokenizer_config.json`, `special_tokens_map.json`) have no SHA-256 anywhere; only `blobId` (the git blob SHA-1, `sha1("blob <size>\0" + content)`). `GET /api/models/<repo>/tree/main` shows the same split (`lfs.oid` is the SHA-256 for LFS entries, `oid` is the git SHA-1). `resolve/` responses for LFS files also send `x-linked-etag` = the SHA-256 and `x-linked-size`.

## (b) GGUF architecture and context length

In the same model response, `gguf: { total, architecture, context_length, bos_token, eos_token, totalFileSize }`. For tiny-llamas: `architecture: "llama"`, `context_length: 128`. It is repo-level (one set of values for the repo), not per file. `config` is `{}` for GGUF-only repos. For safetensors repos, `config.architectures` (e.g. `["LlamaForCausalLM"]`) and `config.model_type` are present, plus `safetensors.parameters` and `safetensors.total`. For a gated repo the same fields are returned without a token (`gated: "manual"`, `config.architectures`, siblings with sizes), but file downloads return HTTP 401 ("Access to model ... is restricted").

## (c) `/api/create` request shape

Blob first (see d), then `POST /api/create`. Streaming NDJSON `{"status": ...}` lines, final `{"status":"success"}`.

GGUF, which worked:
```json
{"model":"hf2ollama-verify-tmp","files":{"stories260K.gguf":"sha256:047bf464...c04b"},"stream":true}
```
Statuses: `parsing GGUF`, `using existing layer sha256:...`, `writing manifest`, `success`. `/api/show` then reported `format: gguf`, `family: llama`, `context_length: 128`, `model_info["general.architecture"] = "llama"`.

Safetensors, which worked (all 7 files, filename to digest, no `from`):
```json
{"model":"hf2ollama-verify-tmp","files":{"config.json":"sha256:...","generation_config.json":"sha256:...","model.safetensors":"sha256:...","special_tokens_map.json":"sha256:...","tokenizer.json":"sha256:...","tokenizer.model":"sha256:...","tokenizer_config.json":"sha256:..."},"quantize":"int8"}
```
Final status: `successfully imported registry.ollama.ai/library/hf2ollama-verify-tmp:latest with 26 layers`. Result: `format: safetensors`, `quantization_level: int8`, `runner: mlx`, `capabilities: ["completion"]`. Omitting `quantize` also succeeds (no re-quantization).

**`quantize` values on 0.40.8 differ from the spec.** The server rejects `q8_0`, `Q8_0`, `q4_K_M` and `f16` with HTTP 400: `unsupported quantize type "q8_0": supported types are int4, int8, nvfp4, mxfp4, mxfp8`. Accepted: `int4`, `int8`, `nvfp4`, `mxfp4`, `mxfp8` (only `int8` and omission were imported end to end). Safetensors models run on the `mlx` runner. Whether this holds on Linux/Windows Ollama is unverified (only macOS available).

Not verified: `quantize` on a GGUF create; unsupported-architecture error behavior.

## (d) Blob upload and HEAD

- `HEAD /api/blobs/sha256:<hex>`: 404 if absent, 200 if present.
- `POST /api/blobs/sha256:<hex>` with the raw file as body: 201 on success. Ollama hashes the body; a wrong digest returns 400 `{"error":"digest mismatch, expected \"sha256:000...\", got \"sha256:2cf24d...\""}`.
- After upload, HEAD returns 200. `/api/create` references blobs by `sha256:<hex>`.
- After the temporary model was deleted, blobs only used by it were removed (404), but the original uploaded `model.safetensors` blob (4 MB) stayed (200), because the converted model uses new layers. There is no API to delete a blob; Ollama may prune it later.

## (e) Minimum Ollama version

The `files` map form of `/api/create` with `HEAD`/`POST /api/blobs` was verified only on 0.40.8; older versions were not available to test. Recommended minimum for v1: 0.40.8 (the verified version), with the app reading `GET /api/version` (`{"version":"0.40.8"}`) and warning below it. The `quantize` vocabulary in (c) is version dependent, so the app should not hard code the list.

## (f) Hugging Face unauthenticated rate limits

Headers on API responses (`/api/models...`): `ratelimit: "api";r=499;t=264` and `ratelimit-policy: "fixed window";"api";q=500;w=300` (500 requests per 300 s window; `r` remaining, `t` seconds until reset). On `resolve/` (downloads, HEAD and ranged GET): `ratelimit: "resolvers";r=2998;t=233`, `ratelimit-policy: "fixed window";"resolvers";q=3000;w=300`. Resolve redirects (302) to a CDN URL; ranged GET with `Range: bytes=0-9` on the redirect target returned 206 with `content-range: bytes 0-9/1185376`, and `accept-ranges: bytes` is advertised. Search responses paginate with a `link: <...&cursor=...>; rel="next"` header.

## Supported architectures provenance

`supported_architectures.json` lists the architectures from Ollama's documented safetensors import support (Llama, Mistral, Mixtral, Gemma, Phi3); only `LlamaForCausalLM` was verified by a real import. Ollama 0.40.8 also runs safetensors through the MLX runner, so the real support set may be wider; the list is data and should be revisited.

## Differences from the spec

1. Section 5: the quantization choices `f16`, `q8_0`, `q4_K_M`, `q4_0` are not accepted for safetensors imports on 0.40.8. Accepted: `int4`, `int8`, `nvfp4`, `mxfp4`, `mxfp8`, or none. Needs a decision.
2. Section 4 step 3: Hugging Face publishes SHA-256 only for LFS files. Non-LFS files (config, tokenizer JSON) can be checked only against the git blob SHA-1 (`blobId`), or not at all. Ollama still needs their SHA-256, which is computed locally while hashing.
3. Naming: HF per-file hash is `siblings[].lfs.sha256`; GGUF metadata is the top-level `gguf` object (repo-level).
