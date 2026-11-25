# Find in Files (rhun extension)

A Find in Files panel in rhun's sidebar: a form under the Explorer, with the results below it.
The built-in find in rhun has only Match Case; this extension adds the rest of
[vshvedov/rhun#119](https://github.com/vshvedov/rhun/issues/119).

- **Form**: Search (Enter runs it), Files to include, Files to exclude, and three toggles: Match
  Case, Whole Word, Regex.
- **Results**: a tree of files with their matches (`line: text`, the match highlighted). Click a
  match to open it at its line.
- **Keys**: Ctrl+Alt+Shift+F focuses the form. (Ctrl+Shift+F is rhun's built-in Find in Files.)
- **Explorer**: right-click a folder, **Find in Folder** fills in *Files to include* with the
  folder (`folder/**`) and focuses the form. The search is left to you, as in VS Code.

## Search syntax

- Files to include / exclude: globs, separated by commas. `src/**` is everything under `src`,
  `*.rs` matches the name at any depth, `{a,b}` is one glob. Without a `/` a glob matches a file
  name at any depth.
- Literal search is rhun's own (`search-text`): it searches unsaved editor text too. Match Case
  off means case-insensitive.
- Regex uses Rust's `regex` syntax (no look-around, no backreferences). It reads the files on disk,
  so unsaved editor text is not searched. Git's ignore rules apply to both.
- Limits: 2000 matches; regex skips files over 2 MB and binary files. The status line says when
  the limit is reached.

## Build and test

`cargo test`, then `sh tests/run.sh` (headless rhun on a fixture project, compared with
`tests/basic.expected`) and `sh tests/check.sh` (the protocol's conformance checker).

## Install

`sh tests/layout.sh DIR`, then copy the built program into `DIR/bin/`, or use rhun's
**Link a Folder (development)** on a folder laid out that way.
