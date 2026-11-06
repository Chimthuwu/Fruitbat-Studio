# Building and Testing (Windows GNU toolchain)

This project targets `x86_64-pc-windows-gnu` (MinGW-w64), **not** MSVC. The build
is finicky because three unrelated linker/toolchain issues collide. This file is
the known-good recipe for the dev box described below.

## TL;DR

```bash
export RUSTUP_HOME=/z/rustup
export CARGO_TARGET_DIR='Z:\lemon-target'
export PATH="/z/mingw64/mingw64/bin:$PATH"
export LIBCLANG_PATH='Z:\LLVM\bin'
export BINDGEN_EXTRA_CLANG_ARGS='--target=x86_64-pc-windows-gnu -resource-dir=Z:/LLVM/lib/clang/20 -mmmx -IZ:/mingw64/mingw64/x86_64-w64-mingw32/include -IZ:/mingw64/mingw64/include'
export RUSTFLAGS="-C link-self-contained=yes -C linker-flavor=ld -C linker=Z:/mingw64/mingw64/x86_64-w64-mingw32/bin/ld.exe"

cargo +stable-x86_64-pc-windows-gnu test -p os-app
```

## Toolchain layout (dev box)

| Thing | Path | Notes |
|---|---|---|
| GNU rustc (stable, 1.99.x) | `/z/z/rustup/toolchains/stable-x86_64-pc-windows-gnu` | NOTE the double-`z` root. `RUSTUP_HOME=/z/rustup`. |
| MinGW gcc 13.2 | `/z/mingw64/mingw64/bin` | MinGW-Builds gcc 13.2.0. |
| GNU ld (binutils 2.47) | `/z/mingw64/mingw64/x86_64-w64-mingw32/bin/ld.exe` | MSYS2 binutils 2.47, **replaces** the 2.39 that shipped with gcc 13.2. |
| LLVM/clang 20 (bindgen) | `/z/LLVM` | `LIBCLANG_PATH=Z:\LLVM\bin`. |
| Rust self-contained CRT + import libs | `<toolchain>/lib/rustlib/x86_64-pc-windows-gnu/lib/self-contained` | Provides `libstdc++.a`, crt2.o, import libs. |

`/mingw64` (single `z`-less path) is **runtime DLLs only** — not a build toolchain.

## Why the RUSTFLAGS are what they are

The three problems, and the single fix that covers all of them:

1. **`.drectve` section** (`-exclude-symbols` from the vendored ASIO SDK) is only
   honoured by GNU ld >= 2.40. The gcc 13.2 that shipped with binutils 2.39
   rejected it (`link: extra operand`).
2. **`raw-dylib` imports** (`NtWriteFile`, `NtReadFile`, `NtSetInformationFile`
   from `link_raw_dylib!`) are emitted correctly by GNU ld >= 2.40, but **dropped
   by LLD/rust-lld** — the resulting binary crashes at startup (TLS callback /
   missing ntdll imports). So we must use GNU ld, not LLD.
3. **LTO plugin ABI mismatch**: GNU ld 2.47 (MSYS2) + gcc 13.2 (MinGW-Builds)
   `liblto_plugin.dll` are incompatible. Linking a trivial C program through
   gcc's `collect2` fails with `collect2.exe: error: ld returned 9`, and the full
   Rust link silently drops ~82 normal CRT imports.

The fix is one flag set: **invoke GNU ld 2.47 directly** (skip gcc/collect2 and
its broken plugin) while letting rustc supply its own CRT + import libs
(`link-self-contained=yes`). `linker-flavor=ld` + `linker=<ld.exe>` does exactly
that:

```text
-C link-self-contained=yes        # rustc provides crt2.o + import libs + libstdc++.a
-C linker-flavor=ld               # invoke the linker directly, no gcc driver
-C linker=.../x86_64-w64-mingw32/bin/ld.exe   # GNU ld 2.47
```

Do **not** use rust-lld here (it drops the ntdll raw-dylib import descriptors),
and do **not** use the stock gcc driver (its collect2 plugin is broken against
binutils 2.47). `link-self-contained`'s granular forms (`+crto`, `-linker`) are
unstable on this stable toolchain — only whole `yes`/`no` works.

## ASIO

`asio-sys` is vendored at `vendor/asio-sys/` (its `layout_tests` are disabled —
the upstream bindgen layout assertions don't match MinGW-w64 headers). The ASIO
SDK source lives in `crates/os-app/asiosdk` (see `.cargo/config.toml` →
`CPAL_ASIO_DIR`), so the build never downloads Steinberg's SDK.

The two ASIO C++ symbols that need Itanium (GNU) mangling — not MSVC — are
gated in `crates/os-app/src/device/asio.rs`:

```rust
ASIOControlPanel    // _Z16ASIOControlPanelv
ASIOGetChannelInfo  // _Z18ASIOGetChannelInfoP15ASIOChannelInfo
```

## Verification

`cargo +stable-x86_64-pc-windows-gnu test -p os-app` runs the full `os-app` test
binary to completion (1081 tests at last count). A green run means the toolchain
is correctly configured.
