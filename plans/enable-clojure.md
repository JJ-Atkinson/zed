# Enable Clojure workflows in Zed

## Goal

Provide a useful Clojure editing workflow with:

- key-bindable structural editing backed by `clojure-lsp`;
- evaluation of the top-level form at the cursor through nREPL;
- a persistent, Clojure-highlighted REPL output file whose newest result is
  first.

Keep the Zed core change language-agnostic and keep Clojure- and nREPL-specific
behavior outside Zed core.

## Implementation status

An initial implementation now exists in two sibling checkouts:

- `zed/` contains the language-agnostic `editor::ExecuteLspCommand` action,
  placeholder expansion, remote transport, and local/remote tests.
- `zed-clojure/` contains top-level runnable detection, the language task,
  `.repl` association, and integration documentation.
- `zed-clojure-eval/` contains the independently installed Babashka evaluator,
  its bbin package metadata, tests, and configuration documentation.

The evaluator is installable from its own repository with `bbin`. Its output is
`.zed/repl.repl`; `.nrepl-port` discovery walks source-file ancestors. Before
evaluating a non-`ns` form, it evaluates the source file's `ns` declaration to
establish aliases, refers, and imports.

## Deliverables

This work has two independently useful halves and will span separate
repositories:

1. **Zed core:** expose parameterized LSP command execution as an editor action.
2. **Clojure tooling:** extend the existing Clojure extension with runnable
   forms and provide a small external nREPL evaluator.

This file is the umbrella plan. Split it into implementation-specific plans
once the action interface and evaluator distribution method are settled.

## Co-development workflow

Keep the repositories and an integration fixture as siblings:

```text
~/code/personal/
├── zed/
├── zed-clojure/
├── zed-clojure-eval/
└── zed-clojure-fixture/
```

Run the locally built editor with an isolated profile:

```sh
cd ~/code/personal/zed
cargo run -- \
  --user-data-dir "$HOME/.local/share/zed-clojure-dev" \
  "$HOME/code/personal/zed-clojure-fixture"
```

In that editor, run `zed: Install Dev Extension` once and select the
`zed-clojure` checkout. A development extension with ID `clojure` overrides the
marketplace installation in this isolated profile. After extension changes,
run `zed: Rebuild Dev Extension`; after Zed core changes, restart the locally
built editor. The development-extension registration survives core restarts.

The local profile at `~/.local/share/zed-clojure-dev` has been pre-seeded for
integration testing:

- `config/settings.json` mirrors the normal Zed appearance and enables Vim
  mode, while adding `~/.local/bin` to task and terminal `PATH` and preserving
  the VS Code Clojure `zprint` formatter choice.
- `config/keymap.json` carries over the VS Code Vim overrides, including
  syntax-node selection, navigation, formatting, code intelligence, Paredit
  commands, evaluation, and `tap>` evaluation.
- `config/tasks.json` adds selection evaluation and `tap>` tasks.
- `extensions/installed/clojure` points at the sibling extension checkout.
- The Bamboo icon and Catppuccin theme extensions are copied from the normal
  profile so the appearance settings resolve without adding extra development
  extensions to the rebuild picker.
- `zed-clojure-eval` is installed in `~/.local/bin`.

The profile can therefore be started without first using
`zed: Install Dev Extension`; use `zed: Rebuild Dev Extension` after subsequent
extension changes.

Useful focused checks:

```sh
# Zed core
cargo test -p editor test_execute_lsp_command
cargo test -p remote_server test_remote_execute_lsp_command
./script/clippy -p editor -p project -p remote_server

# Clojure extension
cargo fmt --check
cargo build --target wasm32-wasip2
cargo clippy --target wasm32-wasip2 -- -D warnings

# Evaluator
cd ../zed-clojure-eval
bb -cp src:test -m zed-clojure-eval-test
```

Completed validation includes the focused core tests, core clippy checks,
extension formatting/build/clippy checks, extension packaging, evaluator unit
tests, Tree-sitter query inspection against the pinned grammar, local `bbin`
installation, and a real JVM nREPL evaluation including source namespace
aliases and Unicode output. The remaining hands-on check is to install the
development extension in the isolated profile and exercise both structural
editing and the runnable keybinding in the graphical editor.

## Existing Clojure extension

The official extension is
[`zed-extensions/clojure`](https://github.com/zed-extensions/clojure), currently
version `0.2.2`.

It currently provides:

- the `prcastro/tree-sitter-clojure` grammar, pinned to a specific commit;
- recognition of `clj`, `cljs`, `cljc`, `cljd`, `edn`, and `bb` files;
- bracket definitions and automatic closing;
- basic indentation;
- syntax highlighting;
- comment injection;
- outlines for common definition forms;
- a `clojure-lsp` adapter;
- use of `clojure-lsp` from `PATH` when available;
- automatic download and caching of the latest stable native `clojure-lsp`
  release otherwise.

It does not currently provide:

- `runnables.scm`;
- language tasks;
- REPL or nREPL integration;
- structural-editing keybindings;
- snippets;
- a `.repl` file association;
- user documentation beyond extension development instructions.

### Extension strategy

Prefer contributing to, or temporarily developing from a fork of, the existing
extension. Do not build a second extension intended to run alongside it.

Language extension data is not compositional. A second extension cannot append
`runnables.scm` or `tasks.json` to the existing `Clojure` language. If two
extensions register a language with the same name, one registration replaces
the other according to extension index/load ownership. Giving the replacement
language another name would create conflicting file detection and duplicate
configuration.

The development workflow should therefore be:

1. Fork `zed-extensions/clojure`.
2. Disable or replace the installed official extension while loading the fork
   as a development extension.
3. Preserve its grammar, queries, and `clojure-lsp` installer.
4. Add the runnable/task functionality to that fork.
5. Submit the result upstream rather than maintaining a permanent replacement.

## Track A: Zed core action

### Current state

Zed already has the underlying functionality:

- `LspStore::execute_lsp_command` accepts a language-server ID, command, and
  JSON arguments.
- `lsp_command_selector::Toggle` discovers commands advertised by language
  servers and executes a selected command.
- local and remote command routing already exist.

The selector action itself accepts no keymap arguments, and no editor action
resolves the current document and cursor into command arguments.

### Proposed action

Add a serializable action with an interface equivalent to:

```json
[
  "editor::ExecuteLspCommand",
  {
    "server": "clojure-lsp",
    "command": "forward-slurp",
    "arguments": ["$uri", "$line", "$column"]
  }
]
```

Initial fields:

- `command`: required string;
- `arguments`: optional JSON values, defaulting to an empty list;
- `server`: optional language-server name.

Initial placeholders:

- `$uri`: the current document's LSP URI;
- `$line`: the zero-based LSP line at the newest selection head;
- `$column`: the zero-based UTF-16 LSP character at the newest selection head.

Only a JSON string that exactly equals a placeholder is substituted. Embedded
string interpolation is out of scope.

### Server selection

1. Resolve the underlying buffer at the newest selection head.
2. Consider only language servers relevant to that buffer.
3. Consider only servers advertising the requested command.
4. If `server` is present, require a matching server name.
5. If `server` is absent, execute only when exactly one server matches.
6. Surface no-match and ambiguous-match errors to the user.

Do not silently select one of several matching servers.

### Local and remote correctness

Resolve `$uri` in the environment where the language server runs. A UI client
may not be able to construct the correct URI for an SSH, collaboration, WSL, or
cross-platform remote project.

Reuse the existing execute-command transport where possible. If necessary,
extend its request context with the source buffer and UTF-16 position so the
remote project can expand placeholders before sending `workspace/executeCommand`.

### Clojure commands enabled

Current `clojure-lsp` advertises these relevant commands:

- `forward-slurp`
- `forward-barf`
- `backward-slurp`
- `backward-barf`
- `drag-backward`
- `drag-forward`
- `raise-sexp`
- `kill-sexp`
- `forward`
- `backward`
- `forward-select`
- `backward-select`

Their cursor argument shape is:

```text
[document-uri, zero-based-line, zero-based-character]
```

### Scope exclusions

- Executing a code action by title or kind.
- Cursor movement based on an LSP command's return value.
- General template expressions.
- Extension-defined actions.
- Clojure-specific action names in Zed core.

### Expected code areas

- `crates/editor/src/actions.rs`
- `crates/editor/src/lsp_ext.rs`
- `crates/editor/src/element.rs`
- `crates/project/src/lsp_store.rs`, if execution context must be transported
- execute-command protobuf definitions, if remote placeholder expansion needs
  additional fields
- editor and remote integration tests

### Acceptance criteria

- A keybinding can execute an advertised command with literal JSON arguments.
- `$uri`, `$line`, and `$column` have correct LSP types and values.
- UTF-16 columns are correct for non-ASCII text.
- An explicit server name selects the intended server.
- an omitted server works when exactly one relevant server advertises the
  command.
- Missing, unadvertised, and ambiguous commands produce visible errors.
- The action works for local and remote projects.
- A fake language server test observes the expected command and arguments.
- A returned or server-initiated workspace edit is applied.

## Track B: Clojure evaluation

### Extension changes

Add to the existing Clojure extension:

- `languages/clojure/runnables.scm`;
- `languages/clojure/tasks.json`;
- `.repl` to the Clojure language's path suffixes;
- setup and usage documentation.

The runnable query should capture top-level evaluable forms and expose their
text as `ZED_CUSTOM_form`, tagged `clojure-eval`. For a top-level
`(comment ...)`, capture each direct body form and omit the outer comment form.

Do not capture arbitrary nested forms. Zed currently stores runnable tasks by
source row, so nested forms beginning on the same row can collide. Direct
comment-block forms are the deliberate exception and should normally be placed
on separate lines.

Avoid treating comments or discarded forms as runnable. Validate the query
against the exact pinned tree-sitter grammar.

The language task should be equivalent to:

```json
[
  {
    "label": "Clojure: Evaluate form",
    "command": "zed-clojure-eval",
    "env": {
      "ZED_CLOJURE_CODE": "$ZED_CUSTOM_form",
      "ZED_CLOJURE_FILE": "$ZED_FILE",
      "ZED_CLOJURE_ROW": "$ZED_ROW",
      "ZED_CLOJURE_COLUMN": "$ZED_COLUMN",
      "ZED_CLOJURE_SELECTION": "${ZED_SELECTED_TEXT:}",
      "ZED_CLOJURE_SYMBOL": "${ZED_SYMBOL:}",
      "ZED_CLOJURE_SNIPPET": "${ZED_CUSTOM_snippet:}"
    },
    "tags": ["clojure-eval"],
    "save": "current",
    "reveal": "never",
    "hide": "on_success",
    "show_summary": false,
    "show_command": false
  }
]
```

Passing captured code through the task environment avoids shell argument
quoting and newline loss.

The keybinding must explicitly specify `"reveal": "never"` for
`editor::SpawnNearestTask`, because that action overrides the resolved task's
reveal strategy. Successful task terminals close via `"hide": "on_success"`.
Failed task terminals remain available in the terminal panel, which can be
opened with <kbd>Ctrl</kbd>+<kbd>`</kbd> or `terminal_panel::Toggle`.

`editor::SpawnNearestTask` accepts a language-agnostic `variables` map. Values
are injected as `$ZED_CUSTOM_<name>` after the ordinary task context and
runnable captures are built. This lets a keybinding select an evaluator
operation while preserving the current selection, nearest form, file, symbol,
row, and column:

```json
[
  "editor::SpawnNearestTask",
  {
    "reveal": "never",
    "use_selection_context": true,
    "variables": {
      "snippet": "test"
    }
  }
]
```

Selection context is opt-in so existing runnable keybindings retain their
standard task-variable semantics.

### Evaluator utility

Provide a small `zed-clojure-eval` Babashka utility, preferably installable via
`bbin`, rather than embedding a machine-specific script path in the extension.

Initial behavior:

1. Read the code, file, and row from environment variables.
2. Find the nREPL endpoint from `.nrepl-port`, with optional environment
   overrides for host, port, and port-file location.
3. Infer the namespace from the source file and default to `user`.
4. Connect to nREPL using Babashka's available bencode support.
5. Send an `eval` operation.
6. Read responses until the `done` status.
7. Collect `out`, `err`, returned values, and evaluation errors.
8. Atomically append a formatted entry to `.zed/repl.repl`.
9. Exit unsuccessfully when connection or evaluation fails, retain the failed
   task terminal, and record the failure in the output file.
10. Merge executable user and project snippet configuration.
11. Give snippet functions ambient editor and endpoint information plus an
    `eval-against-port` callback.

Snippet configuration can provide strings, data maps, or Babashka functions.
It can resolve named targets to fixed ports, port files, or functions of the
ambient context. Each callback opens a connection, evaluates against the
selected endpoint, and contributes its result to the output entry.

The output file is opened manually in a split once. Neither a language
extension nor a task can open or position that editor pane.

### Initial evaluator scope

- Default to JVM Clojure nREPL behavior.
- Allow executable configuration to target ClojureScript-aware endpoints or
  send project-specific switching forms, without claiming persistent
  ClojureScript session management.
- Use one connection per evaluation callback initially.
- Do not add a custom LSP server until the task-based workflow has been
  validated.

### Evaluation acceptance criteria

- Invoking the keybinding inside a top-level form evaluates that form.
- Invoking it inside a direct form in `(comment ...)` evaluates that form
  rather than the outer comment block.
- Multiline forms arrive unchanged.
- The current namespace is used when it can be inferred.
- Standard output, standard error, values, and errors are represented.
- The newest result appears at the end of the output file so Zed's sticky scroll
  keeps it visible.
- Repeated invocations do not corrupt the output file.
- Successful evaluation does not steal editor focus.
- Failed evaluation writes actionable error output and retains its task
  terminal for inspection.
- The `.repl` output file receives Clojure highlighting.
- Snippet functions receive the selection-or-nearest form, top-level form,
  source location, namespace, and discovered nREPL endpoint.
- A keybinding can choose a named snippet without duplicating its code.

## Validation order

1. Use the existing LSP command selector to manually verify the relevant
   `clojure-lsp` commands and argument shapes.
2. Implement and test the core action.
3. Verify structural-editing keybindings against a real Clojure project.
4. Prototype and test the runnable query against representative Clojure,
   ClojureScript, EDN, and Babashka files.
5. Implement the evaluator and test it against a JVM nREPL.
6. Integrate the extension task and output file.
7. Decide whether persistent nREPL connections or inline diagnostics justify a
   follow-up LSP bridge.

## Open decisions

- Whether to split the evaluator into its own repository before proposing the
  extension changes upstream.
- Whether to add ClojureScript session selection in a follow-up.
- Whether a persistent nREPL connection provides enough latency improvement to
  justify a longer-lived helper process.
