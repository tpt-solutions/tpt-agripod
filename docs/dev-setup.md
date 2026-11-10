# Developer setup (Windows 11 host)

Written for: the maintainer, and anyone setting up a Windows workstation for Agripod.

## Toolchains

| Tool | Used for | Install |
|---|---|---|
| Rust (stable, with `rustfmt`, `clippy`) | `edge/` | `rustup` from rustup.rs; `edge/rust-toolchain.toml` pins the channel |
| Go 1.22 or newer | `fleet/` | go.dev/dl, Windows MSI |
| `cargo-deny` | licence and advisory gate | `cargo install cargo-deny --locked` |
| `cargo-zigbuild` or `cross` | aarch64 cross builds (Raspberry Pi, Jetson) | `cross` needs Docker Desktop; `cargo-zigbuild` needs Zig |
| `espup` | ESP32 (Xtensa) Rust toolchain | `cargo install espup --locked`, then `espup install` |
| `espflash` | flashing ESP32 | `cargo install espflash --locked` |
| `go-licenses` | Go licence check (CI tool, not a dependency) | `go install github.com/google/go-licenses@latest` |

Install the tools that run in CI (`cargo-deny`, `go-licenses`) on your workstation so you can reproduce a failing gate locally. They are build tools, not linked into the product, so their own licences are not part of the dependency policy.

## Notes for Windows

- Run Cargo and Go from PowerShell or Git Bash. Use LF line endings. `.gitattributes` enforces this.
- Long paths: enable `LongPathsEnabled` in the registry if the Cargo cache paths fail.
- Serial ports for RS-485 and the ESP32 appear as `COMx`. The Linux serial transport is tested on the Pi, not on Windows.
- The edge daemon targets Linux. Windows builds are for development and tests only.

## Checks

```sh
cd edge
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo deny check

cd ../fleet/orchestrator
go vet ./...
go test -race ./...
```

`go test -race` needs cgo and a C compiler. On Windows, install a MinGW-w64 gcc or run the race job in CI only.
