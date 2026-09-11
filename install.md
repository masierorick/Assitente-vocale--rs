# assistente-rs — Installation Guide

This document explains how to build and install **assistente-rs** from source using Cargo.

## Requirements

Before installing assistente-rs, make sure the following packages are available on your Linux system:

* Rust and Cargo
* ALSA development libraries
* A working audio input device
* A working audio output device

### Install Rust

If Rust and Cargo are not already installed, install them using `rustup`:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

After installation, reload the shell environment:

```bash
source "$HOME/.cargo/env"
```

Verify the installation:

```bash
rustc --version
cargo --version
```

## Build dependencies

On Debian/Ubuntu-based systems, install the required development packages:

```bash
sudo apt install build-essential pkg-config libasound2-dev
```

Depending on the features enabled in the project, additional system libraries may be required.

## Clone the repository

Clone the project and enter its directory:

```bash
git clone <REPOSITORY_URL>
cd assistente-rs
```

Replace `<REPOSITORY_URL>` with the URL of the repository.

## Build

Build the project in release mode:

```bash
cargo build --release
```

The compiled executable will be created at:

```text
target/release/assistente-rs
```

## Configuration

assistente-rs uses a JSON configuration file to define settings such as the browser, music player, wake word, Piper model and Whisper model.

Example:

```json
{
  "always_on_top": false,
  "botname": "MA.R.CO",
  "browser": "vivaldi-stable",
  "deltavolume": 10,
  "layout": "uniwindow",
  "musicplayer": "strawberry",
  "piper_bin": "~/.local/bin/piper",
  "piper_model": "~/.local/share/assistente/piper/voce_marco.onnx",
  "sleep_time": 30,
  "wakeword": "marco",
  "whisper_model": "~/.local/share/assistente/whisper/ggml-medium.bin"
}
```

The configuration should be adapted to the local system.

Paths beginning with `~` are expanded by assistente-rs to the user's home directory.

## Offline speech recognition

For offline speech recognition, assistente-rs uses **Whisper**.

The Whisper model must be available at the path specified by:

```json
"whisper_model": "~/.local/share/assistente/whisper/ggml-medium.bin"
```

The model can be changed according to the available hardware and desired performance.

For example, a smaller model can be used on less powerful systems.

## Offline text-to-speech

For offline text-to-speech, assistente-rs uses **Piper**.

The Piper executable is configured with:

```json
"piper_bin": "~/.local/bin/piper"
```

The voice model is configured with:

```json
"piper_model": "~/.local/share/assistente/piper/voce_marco.onnx"
```

The Piper `.onnx` model must have its corresponding `.onnx.json` configuration file in the same directory.

For example:

```text
~/.local/share/assistente/piper/
├── voce_marco.onnx
└── voce_marco.onnx.json
```

## Online and offline operation

assistente-rs can operate in two modes.

### Online mode

When an Internet connection is available, the assistant can use online speech recognition and text-to-speech services.

### Offline mode

When the system is offline, assistente-rs can use local models:

```text
Speech recognition → Whisper
Text-to-speech     → Piper
```

This allows the core voice interaction to continue without an Internet connection.

## Running the assistant

To run the release build directly:

```bash
./target/release/assistente-rs
```

Alternatively, from the project directory:

```bash
cargo run --release
```

## Installing the executable

If you want to make the executable available system-wide, copy it to `/usr/local/bin`:

```bash
sudo install -m 755 target/release/assistente-rs /usr/local/bin/assistente-rs
```

It can then be started from any directory:

```bash
assistente-rs
```

## Updating and rebuilding

After modifying the source code, rebuild the release version:

```bash
cargo build --release
```

If the executable has been installed in `/usr/local/bin`, update it with:

```bash
sudo install -m 755 target/release/assistente-rs /usr/local/bin/assistente-rs
```

## Troubleshooting

### Check the Rust installation

```bash
rustc --version
cargo --version
```

### Check Piper

```bash
~/.local/bin/piper --help
```

### Check the Piper model

```bash
ls -lh ~/.local/share/assistente/piper/
```

The directory should contain both the `.onnx` model and its `.onnx.json` configuration file.

### Check the Whisper model

```bash
ls -lh ~/.local/share/assistente/whisper/
```

### Rebuild from scratch

If the build behaves unexpectedly:

```bash
cargo clean
cargo build --release
```

## Project structure

A typical assistente-rs source tree contains:

```text
assistente-rs/
├── Cargo.toml
├── Cargo.lock
├── src/
│   ├── main.rs
│   ├── intent.rs
│   ├── vocalrecon.rs
│   ├── tts.rs
│   └── ...
├── ui/
│   └── ...
└── target/
    └── release/
        └── assistente-rs
```

## License

See the `LICENSE` file included with the project.
