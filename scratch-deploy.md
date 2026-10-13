# Scratch Deployment

This setup depends on changes in three sibling checkouts:

```text
zed/
clojure-lsp/
zed-clojure/
```

Do not replace the locally built Zed remote server with an official release
artifact. Remote development is a primary workflow, and the local and remote
Zed binaries must contain the same editor, task, protocol, and LSP-command
changes.

## Build Zed and its remote server

Build an optimized local Zed while enabling only the code that can upload a
locally supplied remote server:

```sh
cd ~/code/personal/zed
cargo build --release \
  --package zed \
  --features remote/build-remote-server-binary
```

For an x86-64 Linux remote, build the remote server as a portable static
binary from the same checkout:

```sh
cargo zigbuild --release \
  --package remote_server \
  --target x86_64-unknown-linux-musl

gzip -c \
  target/x86_64-unknown-linux-musl/release/remote_server \
  > target/x86_64-unknown-linux-musl/release/remote_server.gz
```

Adjust the target triple for other remote platforms.

Always launch this Zed build with the matching remote artifact:

```sh
remote_server="$PWD/target/x86_64-unknown-linux-musl/release/remote_server.gz"
test -f "$remote_server" || exit 1

ZED_COPY_REMOTE_SERVER="$remote_server" \
  target/release/zed \
  --user-data-dir "$HOME/.local/share/zed-clojure-dev"
```

`ZED_COPY_REMOTE_SERVER` makes Zed upload that artifact instead of fetching an
official remote server. For the development release channel it is installed
remotely as:

```text
~/.zed_server/zed-remote-server-dev-build
```

Rebuild both artifacts after protocol or remote-facing Zed changes.

## Build and deploy clojure-lsp

```sh
cd ~/code/personal/clojure-lsp
bb test-lib
bb lint
bb native-cli
```

The resulting `clojure-lsp` must be built for the machine on which it runs.
Build it directly on the remote machine or produce a compatible static binary;
the current local Nix build may reference local `/nix/store` paths.

Install the custom binary on every development host:

```sh
install -Dm755 clojure-lsp ~/.local/bin/clojure-lsp
```

Force Zed to use it, with no download fallback:

```json
{
  "lsp": {
    "clojure-lsp": {
      "binary": {
        "path": "~/.local/bin/clojure-lsp"
      }
    }
  }
}
```

This can live in the isolated profile or the project's `.zed/settings.json`.
After replacing the binary, run `editor: Restart Language Server`.

## Build the Clojure extension

```sh
cd ~/code/personal/zed-clojure
cargo fmt --check
cargo build --target wasm32-wasip2
cargo clippy --target wasm32-wasip2 -- -D warnings
bb -cp tools/zed-clojure-eval/src:tools/zed-clojure-eval/test \
  -m zed-clojure-eval-test
```

Install this checkout with `zed: Install Dev Extension`, then use
`zed: Rebuild Dev Extension` after extension, task, or query changes.
