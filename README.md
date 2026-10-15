# Furkst

A fast, friendly desktop git client for Windows and macOS, written in Rust on top of
[gitoxide](https://github.com/GitoxideLabs/gitoxide) (`gix`) with an [egui](https://github.com/emilk/egui) interface.
It opens large repositories quickly (the first page of git/git's 85,000-commit history in about 20 ms) and stays
small: one executable, no runtime to install.

```sh
cargo run --release                 # restores the previous session
cargo run --release -- path/to/repo # also opens the given repositories
cargo test                          # unit + end-to-end tests against throwaway repositories
```

Requires the latest stable Rust (edition 2024, `rust-version = 1.99`) and `git` on `PATH`.

## Features

- **Repositories**: tabs with session restore, a start page with recent repositories and named groups, open,
  init and clone with live progress, a Quick Launch palette (Ctrl+P / Cmd+P).
- **History**: commit graph with branch, tag and remote badges, avatars of GitHub users (optional), virtualized
  scrolling through any size of history, "show only this branch", and a search (Ctrl+F) over hashes, authors,
  messages and ref names, optionally also the paths each commit changed, stepping through matches with Enter.
- **Commit details**: a Commit tab with every changed file opening in place, a Changes tab with the files in a
  folder tree beside the diff, and a File Tree tab with the whole tree and a file viewer.
- **Diffs**: syntax highlighting, unified or side by side (scrolling together), context lines, ignore whitespace,
  image diffs, Git LFS files by content, stepping between changes, selectable text, "Reveal line in" the installed
  editor (VS Code and relatives, JetBrains IDEs, Zed, Sublime Text, Notepad++), and a separate Changes window.
- **Local changes**: unstaged and staged lists, staging, unstaging and discarding whole files, hunks or single
  lines, commit with templates and amend.
- **Branches and remotes**: checkout by double-click, create, rename, delete (the local branch, its remote branch
  or both), track upstreams, fetch, pull (merge or rebase), push with force-with-lease, tags, remotes, background
  auto-fetch, drag a branch onto the current one to merge or rebase.
- **Merging**: fast-forward, merge commit, squash and merge, rebase and merge; stash and merge (or pull) when
  uncommitted changes are in the way, putting them back afterwards; a Conflicts window that resolves conflict by
  conflict (accept current, incoming or both, single lines, or typed by hand) or in an external merge tool;
  cherry-pick, revert, reset; continue or abort anything in progress.
- **Interactive rebase**: reorder, pick, reword, squash, fixup and drop, from any commit or the root, keeping
  uncommitted changes aside.
- **More**: stashes (including untracked files), file history with each version's diff, blame, the reflog,
  git-flow, Git LFS, submodules and worktrees.
- **Look and feel**: light, dark or system theme, readable colors checked against WCAG contrast, and any
  installed font for the interface and for code.

## Building executables

| Platform | Command (run on that platform) | Output |
| --- | --- | --- |
| Windows | `.\scripts\build-windows.ps1` | `dist\furkst.exe`: a single standalone file (static C runtime, no console window) |
| macOS | `./scripts/bundle-macos.sh` | `dist/Furkst.app`: a universal binary for Apple Silicon and Intel, plus a `.zip` |

macOS apps must be linked on a Mac, so either run the script on one or let CI do it:
`.github/workflows/release.yml` checks, tests and builds both platforms on every push, and attaches the builds to
a GitHub release when a `v*` tag is pushed.

The macOS app is ad-hoc signed but not notarized, so the first launch is blocked by Gatekeeper. Right-click the app
and choose **Open**, or run `xattr -dr com.apple.quarantine Furkst.app`.

Both platforms need `git` installed. On macOS, Furkst also looks in `/opt/homebrew/bin`, `/usr/local/bin` and
`/usr/bin`, since apps opened from Finder don't inherit the shell's `PATH`; set `FURKST_GIT` to override.

## Development

Iterate with `cargo check` and `cargo test`; CI also runs `cargo clippy --all-targets -- -D warnings`.

- `scripts/make-demo-repos.ps1` builds a set of demo repositories in a few seconds (changes, conflicts, rebase,
  merge styles, git-flow, LFS, submodules and worktrees) to try every feature by hand.
- `cargo run --release -p furkst-core --example profile -- <repository>` times the operations the interface relies
  on, to check performance on large repositories.
- Set `FURKST_SETTINGS` to a file path to run with throwaway settings instead of your own.

| Crate | Purpose |
| --- | --- |
| `crates/furkst-core` | Git backend. Plain owned data types, no UI. Every change to a repository is an `Operation` value with a single `run` dispatch. |
| `crates/furkst-app` | The egui application (`furkst` binary). Views emit `Action`s; the tab applies them and runs git work on background threads. |

### gitoxide and `git`

| Done in-process with gitoxide | Delegated to the `git` executable |
| --- | --- |
| Open / init, refs, remotes, tags, stashes, submodules | Clone, fetch, pull, push |
| History walk, search, file history, the commit graph | Checkout / switch |
| Status, commit and working-copy diffs (with filters such as `autocrlf`) | Merge, rebase, cherry-pick, revert, reset |
| Stage / unstage / discard files, hunks and lines | Stash push / apply / pop / drop |
| Commit and amend | Continue / abort of operations in progress |
| Create / rename / delete branches; create / delete tags | Remotes, upstream tracking, submodules, LFS, worktrees |
| Blame | Commits when hooks, signing or a merge in progress need git's own behavior |

Network operations go through `git`, so existing credential helpers and SSH setups work unchanged. `cli.rs` is the
only place that runs it.

## Acknowledgements

Furkst stands on [gitoxide](https://github.com/GitoxideLabs/gitoxide), [egui](https://github.com/emilk/egui),
[syntect](https://github.com/trishume/syntect) with the language definitions and themes of
[bat](https://github.com/sharkdp/bat) (through [two-face](https://codeberg.org/CosmicHarper/two-face)),
[fontdb](https://github.com/RazrFalcon/fontdb) and [ureq](https://github.com/algesten/ureq), among the crates listed
in `Cargo.lock`.

## License

MIT; see [LICENSE](LICENSE).
