# Third-Party Components

The MIT license at the repository root covers this project's code, not its
dependencies, downloaded models, or user-provided binaries.

| Component | Upstream | License |
| --- | --- | --- |
| sherpa-onnx | https://github.com/k2-fsa/sherpa-onnx | Apache-2.0 |
| ONNX Runtime | https://github.com/microsoft/onnxruntime | MIT |
| llama.cpp, when supplied | https://github.com/ggml-org/llama.cpp | MIT |

Rust dependencies are selected by `Cargo.lock`; each
crate's package metadata and distributed license files specify its terms. These
three native components are not a complete transitive dependency inventory.

Models downloaded through the UI or supplied locally have independent licenses.
Check the original model repository, model card and any training-data restrictions
for the exact model version before use or redistribution. A download URL does not
grant rights under this project's MIT license. This also applies to translation
GGUFs, denoising and speaker models, and any audio supplied by clients.

For binary releases, retain the license texts, copyright notices and applicable
NOTICE files from the exact native archives and Rust dependencies used. Include
those materials in the release alongside the project license. Record prebuilt
llama.cpp versions, backends and archive checksums in release notes. The packaging
script can copy a prepared notices directory; see PACKAGING.md. It does not derive
licenses from binary filenames or verify model redistribution rights.
