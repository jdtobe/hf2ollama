# FAQ

hf2ollama is pre-release and the GUI is under construction. Answers below describe the designed and verified behavior; where something is only planned, it says so.

## Why use the Ollama API instead of writing into the Ollama store?

The server owns its store. If hf2ollama copied files into `~/.ollama/models` it would break as soon as your Ollama used a different models directory, ran in a container, or ran on another machine, and it could break when Ollama changes its store format. Uploading blobs (`/api/blobs`) and calling `/api/create` works with any Ollama you can reach over HTTP.

## GGUF or safetensors: what is the difference here?

- **GGUF** repos contain ready-to-run files, usually in several quantizations. You pick a file (split shards count as one option) and Ollama registers it as is.
- **Safetensors** repos contain full-precision weights plus `config.json` and tokenizer files. They are downloaded in full, uploaded together, and Ollama builds the model from them, optionally quantizing. They are only importable when the architecture is on the supported list, which is bundled data. Only `LlamaForCausalLM` was verified with a real import; the other listed architectures follow Ollama's documented support and are unverified.

## Why are the safetensors quantization choices `none`, `int8` and `int4`?

On the Ollama version we verified (0.40.8, on macOS only), `POST /api/create` for safetensors rejects `q8_0`, `q4_K_M` and `f16` with HTTP 400 and accepts `int4`, `int8`, `nvfp4`, `mxfp4` and `mxfp8`, or no `quantize` at all. Safetensors models ran on Ollama's `mlx` runner. Only `int8` and no quantization were imported end to end; `int4` is accepted by the server but was not imported in verification. The list the app offers is bundled data, not hard-coded logic.

On Linux and Windows, behavior is unverified. The accepted values may differ. The app shows any Ollama rejection as is, and does not retry with other values. If you try it on another platform, an issue with the Ollama version and the message is useful.

## Why is a repository marked "unsupported"?

A repo is unsupported when it has no GGUF files and its safetensors architecture is not on the supported list, or when it contains only other formats such as PyTorch `.bin` or ONNX. hf2ollama does not convert models (there is no Python or llama.cpp step), so it tells you why before downloading anything and disables the import button. Where search finds related GGUF repos, it suggests them.

## What happens to downloaded files? How do I keep them?

Files are downloaded into a per-job folder in the staging folder (the OS cache directory by default; configurable). By default they are deleted after a successful import. Turn off "Delete downloaded files after import" to keep them; they then appear under Cached files in Settings, with a Clear button.

Cancelling a job always deletes its partial files. A failed job keeps resumable partials for 7 days if "Keep partial downloads for 7 days" is on, otherwise it deletes them. A startup sweep removes orphaned job folders and expired partials.

## Is it safe to read a model card inside the app?

The model card (the repository README) is shown as a restricted subset of Markdown. Raw HTML is dropped, images are never downloaded, and only plain `http` and `https` links are kept; clicking one opens your browser. The text is size-capped (256 KiB) and the amount drawn is bounded, so a huge or hostile README is cut off ("truncated") instead of slowing the app down. The Hugging Face token is not sent to other hosts if the README URL redirects.

## Why does Delete ask twice, and why is the second click sometimes ignored?

Delete in Installed (and Clear in Settings) works in two steps: the first click arms the button, the second one confirms. The second click is ignored for the first 0.6 seconds after arming, so a quick double-click can never delete a model, and a held-down Enter or Space never confirms. The armed state expires after 4 seconds. Delete removes the model through the Ollama API.

## What is portable mode?

Put an empty file named `portable` next to the executable. Settings, import history and logs are then stored beside the executable, and the default staging folder is there too. Remove the file to go back to the OS config, cache and log directories. This lets you run the app from a USB drive without touching the host profile. The Hugging Face token is still kept in the system keychain, not beside the executable.

## Can I use a non-default models directory or a remote Ollama?

Yes, that is the point of using the API. Set the Ollama address in Settings (default `http://localhost:11434`) and use Test. The models directory shown in the app is whatever the server reports. If Ollama runs on another machine, set the hardware override in Settings so fit estimates use that machine's memory rather than yours. Staging still happens on the machine running hf2ollama, and files are uploaded over HTTP, so large models cost network time.

## Known limitations

- **Orphan blobs.** Ollama has no blob-delete API. If an import fails after upload, an unused blob can remain in Ollama's store (we observed 4 MB in testing). Ollama prunes unreferenced blobs on restart. hf2ollama documents this rather than working around it.
- **No cross-restart download resume in v1.** Pause and resume work within a session, and partial files can be kept, but a job is not automatically restored after the app restarts.
- **VRAM detection** covers NVIDIA GPUs and Apple Silicon unified memory only. Otherwise VRAM is shown as unknown and you can enter it manually. Fit results are estimates, not guarantees.
- **Hash coverage.** Hugging Face publishes SHA-256 only for LFS files. Small config and tokenizer files are checked against a git SHA-1 only.
- **Platforms.** The Ollama and Hugging Face behavior was verified on macOS only, and the UI prototype was checked on macOS only.
- **Accessibility.** egui has weaker accessibility support than native toolkits.
- **Minimum Ollama version.** 0.40.8 is the only verified version; older versions were not tested.
