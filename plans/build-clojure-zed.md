# Build the Clojure-enabled Zed artifacts

## Goal

Build an optimized local Zed and a matching portable remote server from this
checkout. After both artifacts are validated, decide how to install and manage
them from `/etc/nixos`.

The local and remote binaries must come from the same source tree because this
work changes the remote protocol, task context, and LSP command execution.
Never pair this local build with an official Zed remote-server artifact.

## Current status

- [x] The focused editor and remote-server tests pass.
- [x] The relevant Zed crates pass `./script/clippy`.
- [x] `cargo-zigbuild` and Zig are available.
- [x] The custom static `clojure-lsp` is installed separately.
- [x] The Clojure extension and `zed-clojure-eval` are separate repositories.
- [x] Build optimized local Zed.
- [x] Build the matching static remote server.
- [x] Compress and validate the remote-server artifact.
- [x] Record artifact sizes, hashes, and source provenance below.
- [ ] Launch Zed with the isolated Clojure profile.
- [ ] Confirm local structural editing and evaluation.
- [ ] Confirm remote structural editing and evaluation.
- [ ] Design `/etc/nixos` packaging and deployment.

## Constraints

- Do not clean `target/` unless an incremental build cannot be repaired.
- Do not modify or remove the unrelated `crates/gpui_playground/`.
- Do not commit the current Zed changes.
- Use the isolated profile at
  `~/.local/share/zed-clojure-dev`.
- Build the remote server from this checkout rather than downloading one.
- Keep the final remote-server binary portable across x86-64 Linux hosts.

## Record source provenance

The checkout intentionally contains uncommitted work, so the base commit alone
does not identify the build. Record both the commit and a hash of the tracked
diff:

```sh
cd ~/code/personal/zed

git rev-parse HEAD
git diff --binary | sha256sum
git status --short
```

Relevant untracked planning and playground files are not build inputs. Review
the status before building to ensure no unexpected source file is present.

## Build optimized local Zed

Enable only the feature needed to upload a locally supplied remote-server
binary:

```sh
cd ~/code/personal/zed

cargo build --release \
  --package zed \
  --features remote/build-remote-server-binary
```

Expected artifact:

```text
target/release/zed
```

The local executable is built for this NixOS system. Do not assume it is
portable until its runtime libraries and asset requirements have been checked.

## Build the matching remote server

Build a static musl executable for an x86-64 Linux remote:

```sh
cd ~/code/personal/zed

env -u ZSTD_SYS_USE_PKG_CONFIG \
  cargo zigbuild --release \
  --package remote_server \
  --target x86_64-unknown-linux-musl
```

Expected artifact:

```text
target/x86_64-unknown-linux-musl/release/remote_server
```

The Nix development environment sets `ZSTD_SYS_USE_PKG_CONFIG=1`, which selects
the host's shared-only zstd package and makes static linking fail at the final
link step. Unsetting it makes `zstd-sys` compile its bundled static library for
the musl target. This does not modify Zed source.

Compress the artifact in the format expected by Zed's upload path:

```sh
gzip -n -c \
  target/x86_64-unknown-linux-musl/release/remote_server \
  > target/x86_64-unknown-linux-musl/release/remote_server.gz
```

`gzip -n` omits the original filename and timestamp so repeated compression of
the same binary is reproducible.

## Validate the artifacts

```sh
test -x target/release/zed
test -x target/x86_64-unknown-linux-musl/release/remote_server
test -s target/x86_64-unknown-linux-musl/release/remote_server.gz

file target/release/zed
file target/x86_64-unknown-linux-musl/release/remote_server
ldd target/x86_64-unknown-linux-musl/release/remote_server || true
gzip -t target/x86_64-unknown-linux-musl/release/remote_server.gz
target/x86_64-unknown-linux-musl/release/remote_server version

sha256sum \
  target/release/zed \
  target/x86_64-unknown-linux-musl/release/remote_server \
  target/x86_64-unknown-linux-musl/release/remote_server.gz

du -h \
  target/release/zed \
  target/x86_64-unknown-linux-musl/release/remote_server \
  target/x86_64-unknown-linux-musl/release/remote_server.gz
```

The remote server should report as a statically linked x86-64 ELF executable
and should not have dynamic dependencies.

## Launch the isolated build

```sh
cd ~/code/personal/zed

remote_server="$PWD/target/x86_64-unknown-linux-musl/release/remote_server.gz"
test -f "$remote_server" || exit 1

ZED_COPY_REMOTE_SERVER="$remote_server" \
  target/release/zed \
  --user-data-dir "$HOME/.local/share/zed-clojure-dev"
```

For the development release channel, the uploaded server is installed remotely
as:

```text
~/.zed_server/zed-remote-server-dev-build
```

## Graphical validation

Use `~/code/personal/zed-clojure-fixture` for the local checks:

- Start all targets with `script/start-all`.
- Confirm `z` evaluates JVM and ClojureScript forms.
- Confirm `Space r l` loads the current file.
- Confirm `Space r 0`, `Space r 1`, and `Space r 2` switch targets.
- Confirm the managed status remains first and pending evaluations show
  `#_:WAITING`.
- Confirm structural commands, including forward/backward sexp-or-up.

Repeat the structural and evaluation checks in an SSH remote project. Confirm
that `$uri`, `$line`, and `$column` are resolved on the remote host and that the
custom remote `clojure-lsp` executable is used.

## Artifact manifest

Fill this in after a successful build:

```text
Base commit: 2c99f547ccf6843adca27fc2b17576991d70925e
Tracked diff SHA-256: 723065b17e5d78ed56814c1de2c8cc55f9b9330be2d15f1b287a816b49d30e75
Build date: 2026-10-10
Build host: nixos, Linux 7.1.2 x86_64
Toolchain: rustc/cargo 1.98.1, Zig 0.16.0, cargo-zigbuild 0.23.4

Local Zed:
  Path: target/release/zed
  Size: 2,124,513,664 bytes (2.0 GiB)
  SHA-256: 717d50b32487a7d62d88fb72112746b89319b617b06c8439ea95f755a02cf388
  Runtime dependencies: dynamic NixOS build; 14 direct ELF dependencies

Remote server:
  Target: x86_64-unknown-linux-musl
  Path: target/x86_64-unknown-linux-musl/release/remote_server
  Size: 737,474,176 bytes (704 MiB)
  SHA-256: 347376327a1ad1793f31b82a4011760b476e65064841aa310cda36e787680e37
  Embedded version: 2c99f547ccf6843adca27fc2b17576991d70925e

Compressed remote server:
  Path: target/x86_64-unknown-linux-musl/release/remote_server.gz
  Size: 168,539,857 bytes (161 MiB)
  SHA-256: df1681757778a65cf3091d70e07552febea1c017ff04c34c859b9e5c48e82160
```

## `/etc/nixos` deployment

Do not choose the deployment mechanism until the artifacts have been built and
inspected.

The follow-up must decide:

- whether to package the local executable from source in Nix or import the
  prebuilt artifact;
- how to provide Zed's runtime assets and dynamic libraries;
- where the matching compressed remote server should live in the Nix store;
- how the launcher should set `ZED_COPY_REMOTE_SERVER`;
- whether the isolated profile remains explicit or gets a dedicated wrapper;
- how upgrades atomically replace both matching artifacts;
- how to retain or roll back a previously working pair;
- whether artifact hashes and build provenance belong directly in
  `/etc/nixos` or in a separate pinned flake input.

The NixOS configuration must preserve the requirement that local Zed and its
remote server are updated as one versioned unit.
