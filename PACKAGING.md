# Native release packaging

Run `./package.sh` with a native Rust toolchain. Builds use `--locked` and
explicitly select the toolchain's host target; cross compilation is unsupported.
Use `--no-strip` to retain symbols.

All platforms produce a `.tar.gz` archive using `tar`, including Windows under
Git Bash. The packager does not require `zip`. Extract with `tar -xzf <archive>`.

Set `LLAMA_BIN_DIR` to the directory containing the supplied `llama-server`.
Optionally set `LLAMA_LIB_DIR` to its runtime library directory (which may be
the same directory). All shared libraries in `LLAMA_LIB_DIR` are included,
including dynamic backends; their local transitive dependencies are inspected.
Without these inputs, stale target-directory llama executables are not bundled.

GitHub Actions prepares `llama-server` in a separate matrix job using
`sjl623/llama.cpp`'s `STQ_0` fork for HY-MT2 STQ models. The exact source commit
is pinned in `.github/workflows/build.yml`. Only the `llama-server` target and
its dependencies are built, with `BUILD_SHARED_LIBS=OFF`. These CPU builds
disable GPU backends, OpenMP, OpenSSL, and the Web UI; system libraries remain
platform requirements. Windows uses the static MSVC runtime.

The finished binary, license, and source revision are cached by runner image
label, architecture, source commit, and workflow hash. Cache hits skip source
checkout and compilation. Each run uploads the cached archive as an artifact;
the test job downloads it and sets `LLAMA_BIN_DIR`. After tests pass, the job
runs `package.sh` to build a native release package containing that binary,
and includes the fork's license and source revision in `third-party-notices/`.
Each platform's `.tar.gz` package is uploaded as a workflow artifact.

On Linux, run trusted packages with the package directory in `LD_LIBRARY_PATH`
when their embedded search paths do not already find sibling libraries. For
example, from the package directory:

```sh
LD_LIBRARY_PATH="$PWD${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" ./sherpa-onnx.server
bash ./check.sh .
```

When present beside the packager, `check.sh` is bundled on Linux and must pass
before archiving. It checks every top-level ELF file against the current host,
including staged backends, missing libraries, ELF interpreters and glibc versions.
It uses `ldd`, so only check trusted binaries. This does not establish compatibility
with other distributions, CPUs, GPU drivers, or backend runtime configurations.
macOS and Windows packages also need testing on their intended hosts; macOS
library install names and search paths may require upstream build configuration.
System libraries (including OpenSSL) are not automatically collected from the
host. No special OpenSSL version or distribution restriction is imposed.

README.md, THIRD_PARTY.md and the project's MIT LICENSE are copied into the
package. Set `THIRD_PARTY_NOTICES_DIR=/path/to/notices` to include prepared license
texts and notices under `third-party-notices/`. Obtain these from the exact native
archives and Rust dependencies used; this directory is a release input, not a
generated license audit. See THIRD_PARTY.md. The script does not infer those terms
or verify that the supplied notices are complete. FST data and the old RELEASE.txt
are not included.
