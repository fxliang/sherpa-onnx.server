# sherpa-onnx.server

A WebSocket speech recognition server built with Rust and sherpa-onnx. It provides
streaming and offline recognition, model management in a terminal interface,
optional denoising and speaker verification, and translation through llama-server.
The interface supports English, Simplified Chinese, Traditional Chinese, Japanese,
Korean, Russian, Spanish, Brazilian Portuguese, French and German.

## Build

Native Linux, Windows and macOS builds are the intended targets. Linux is verified
locally; the GitHub Actions workflow checks all three platforms when run. Cross
compilation is not supported by the release script.

Install a current stable Rust toolchain and a native C/C++ build toolchain. Local
verification uses Rust 1.92.0. On Ubuntu 24.04, install the native prerequisites:

```sh
sudo apt-get install build-essential pkg-config libssl-dev liblzma-dev libbz2-dev
cargo build --locked --release
```

On Windows, use Rust's MSVC toolchain with Visual Studio C++ Build Tools and the
Windows SDK. On macOS, install the Xcode command-line tools. Then run the same
Cargo command in a terminal. The executable is `target/release/sherpa-onnx-server`
(`sherpa-onnx-server.exe` on Windows).

sherpa-onnx 2.0.1 uses static native libraries on Linux/macOS and shared libraries
on Windows. Its dependency build script downloads matching prebuilt libraries;
first builds require network access. To supply compatible native libraries locally,
set `SHERPA_ONNX_LIB_DIR=/path/to/sherpa-onnx/lib`. Static native linking still
leaves platform runtime dependencies such as glibc and OpenSSL on Linux.

## First Run

Start the terminal interface, initially listening only on the local machine:

```sh
cargo run --locked --release -- --host 127.0.0.1
```

1. Press `d` to download a supported ASR model, or `i` to import one locally.
2. Highlight the installed model and press `s` to select it.
3. Press `r` to start the WebSocket server on port 8765.
4. Press `o` for network, translation, denoising and language settings; `?` shows help.

VAD, punctuation, denoising and speaker verification require their corresponding
models and settings. A translation GGUF alone does not configure an ASR model.

Configuration and models default to `~/.sherpa-onnx.server/models/`, with settings
in `config.json`. `--models-dir /path/to/store` changes the storage root; settings
then live in `/path/to/store/models/config.json`. Configure and select an ASR model
once before headless startup:

```sh
cargo run --locked --release -- --start --host 127.0.0.1
```

Use a WebSocket client following the protocol below to test a running server
with your own audio. Audio files are not included in the source repository.
Use `--help` on the server for additional options.

## Network and Logs

The default host is `0.0.0.0` for Android/LAN clients, with mDNS and UDP discovery.
Use `--host 127.0.0.1` for local access; loopback listeners disable discovery.
WebSocket connections have no built-in authentication or encryption. Run on a
trusted LAN, or place the server behind an authenticated TLS proxy with direct
access to its port restricted. Speaker verification filters speech; it does not
authenticate network clients.

Normal logs omit recognized and translated text. `--log-level debug` opts into
content logging, which may contain private speech.
A configured GitHub token is sent only to HTTPS `github.com` and `api.github.com`,
not third-party mirrors. Config files use owner-only permissions on Unix; Windows
uses inherited filesystem ACLs. Do not publish configs, tokens or voice samples.

## Prebuilt llama-server

This project does not build llama.cpp. Supply already compiled files if you want
the translation backend copied next to the server:

```sh
LLAMA_BIN_DIR=/path/to/prebuilt/bin \
LLAMA_LIB_DIR=/path/to/prebuilt/lib \
cargo build --locked --release
```

`LLAMA_BIN_DIR` directly contains `llama-server` (`llama-server.exe` on Windows).
`LLAMA_LIB_DIR` is optional: omit it for a backend with no additional runtime
libraries. Only shared libraries directly inside that directory are copied,
including dynamically loaded backends. Set both variables to the same directory
for a flat release bundle. Relative paths are resolved from this project's root.
Use clean directories containing the required runtime files. Existing files in
Cargo's output directory are not deleted automatically.

After selecting an ASR model, enable translation with a compatible GGUF:

```sh
cargo run --locked --release -- --start --host 127.0.0.1 \
  --translate \
  --translate-model /path/to/translation.gguf \
  --llama-server /path/to/prebuilt/bin/llama-server \
  --translate-target en
```

If libraries are separate, add `--llama-lib-dir /path/to/prebuilt/lib`. Low-bit
models identified by the runtime use CPU inference; for compatible GPU models
and backends, configure offload with `--translate-gpu-layers`.

## WebSocket Protocol

Connect to `ws://HOST:8765/v1/asr`. Send a text control frame to create a session:

```json
{"type":"start","sampleRate":16000,"silenceMs":3000}
```

Then send binary frames containing mono, little-endian IEEE 754 float32 samples
(normally normalized to -1 through 1), without a WAV header. Use the model's sample
rate where possible and set `sampleRate` to the actual rate of the supplied audio.
The recognizer can resample, but preprocessing such as VAD has its own rate
requirements. Audio sent
before `start` is ignored. Finish with `{"type":"end"}` and wait for `ended`;
`{"type":"cancel"}` or `{"type":"stop"}` cancels the session.

Responses are JSON text frames:

| Type | Fields and Meaning |
| --- | --- |
| `partial` | `text`: intermediate recognition, or original ASR text before translation |
| `final` | `text`, `translated`: committed result; `translated` is true only when translation produced the result |
| `error` | `code`, `message`: an operation failed |
| `ended` | Session finished |

When enabled, translation first emits the original text as `partial`, then the
translated text as `final` with `translated: true`. If translation is skipped,
`final` contains the original with `translated: false`. If translation fails, the
server emits an `error`, then falls back to the original `final`. There is no
separate `translation` event. A session may produce multiple final segments.

## Packaging and Development

`cargo test --locked` runs the regression tests. [PACKAGING.md](PACKAGING.md)
describes native release packaging with `bash ./package.sh`. Packaging accepts
the same llama input variables and requires Python 3 plus platform inspection
tools. `--no-llama` creates an ASR-only package. Generated archives go in `dist/`.

`local-only/` is ignored and holds local experiments, auxiliary tools and data
that are not part of the published source. It is not required to build or package.

## License

Project code is licensed under [MIT](LICENSE). Dependencies and downloaded models
retain their own terms; see [THIRD_PARTY.md](THIRD_PARTY.md). Models, sample audio
and llama.cpp binaries are not distributed in this source repository.
