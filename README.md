> [!NOTE]
> **This is the new Rust rewrite of `machine-sync`.**
> It no longer depends on `lsyncd`, applies nested `.gitignore`s correctly, can sync to arbitrary destinations, supports a config file, persistence/pause/resume, and richer notifications.
> The original bash + `lsyncd` proof of concept lives on the [`legacy-lsyncd`](../../tree/legacy-lsyncd) branch.

# msync

`msync` live-syncs a directory to one or multiple machines over `rsync` (an SSH connection is enough; no daemon is required on the remote).
It honors your `.gitignore`s at any depth, and sends a notification listing the changed files on every sync.

## Usage

```
msync              # same as msync status
msync status  [-a]
msync start   [<name|group|host[:path]>...]
msync stop    [<id|name>...] [--all]
msync pause   [<id|name>...] [--all]
msync resume  [<id|name>...]
msync restart [<id|name>...]
msync logs    <id|name> [-f]
```

### Targets

A target is `host`, `host:/abs/path`, `host:~/rel`, or `host:rel`.
A bare `host` defaults to `host:~/<basename-of-source-dir>`.

```
msync start laptop server:/srv/site    # sync CWD to two destinations
msync start --dir ~/code/app server    # sync a specific directory
```

To sync one directory to several destinations, pass them all in a single `start` command (as above).
They become one sync with multiple targets.

### One sync per directory

A directory can have only one active sync.
Running `msync start` again for a directory that is already being synced **replaces** the existing sync (the old one is stopped and removed, and a new id is assigned).
To sync the same directory to different destinations at once, list them in one `start` command rather than starting several.

### Config file

Define reusable syncs in a global config at `~/.config/machine-sync/config.toml`, or a per-project `.msync.toml` discovered by walking up from the current directory.

```toml
# default = "project"        # optional; otherwise the first-defined sync is the default

[sync.dotfiles]
dir = "~/dotfiles"
targets = ["laptop", "server:/etc/dots"]

[sync.project]
dir = "~/code/project"
targets = ["server:~/project"]
debounce_ms = 300            # optional; coalescing window (default 700)

[group.work]
members = ["dotfiles", "project"]
```

Per-sync options: `dir`, `targets`, `delete`, `dry_run`, `notify`, `rsync_args`, `debounce_ms`, and `default`.
They can be set in either the global or the project config.

```
msync start                 # in a repo with .msync.toml: start its first-defined sync
msync start project         # start a named sync
msync start work            # start a group
msync start --all           # start every sync in the resolved config
```

Syncs are recorded in a registry and survive a reboot; bring them back with `msync resume`.

## Tips and features

### Notifications

`msync` sends a notification with the changed files on each sync.
It auto-detects the best backend:

- **Linux desktop** via D-Bus (like `notify-send`).
- **Terminal** escape sequences (`OSC 9` / `OSC 777`) for terminals that support them (WezTerm, iTerm2, …).
- **WSL** toast via `wsl-notify-send`, with a built-in PowerShell balloon fallback when it isn't installed.

### Ignoring files

`msync` applies your `.gitignore` rules.
You can add extra patterns in a `.msyncignore` file (same syntax as `.gitignore`).

To **force-sync** a file that would otherwise be ignored, un-ignore it via a negation in `.msyncignore`:

```
# .msyncignore
!build/keep-this.txt
```

Alternatively, add a `# nomsyncignore` marker line to `.gitignore`; any pattern listed *after* it is force-synced.

> **Legacy compatibility:** the old `.ldignore` file and the `# noldignore` marker are still recognized and behave the same way.

### Deletions

By default `msync` never deletes files on the remote, even when you delete them locally.

### Multiple targets

When a sync has several targets, `msync` runs `rsync` to all of them **in parallel**, so one slow or unreachable host does not hold up the others.

### Debouncing

Rapid bursts of file changes are coalesced into a single sync.
The window defaults to 700 ms and can be tuned per sync with `debounce_ms` in the config or the `--debounce <ms>` flag on `msync start`.

### SSH access

`msync` runs `rsync` over SSH and relies entirely on your SSH configuration and does not set any SSH options itself.
Use key-based authentication (and an `ssh-agent` if your key has a passphrase): a background sync has no terminal, so a host that prompts interactively will cause its `rsync` to hang.
If a target is temporarily unreachable, the sync simply fails for that batch and is retried on the next file change.

On medium/high-latency links, enable [SSH multiplexing](https://en.wikibooks.org/wiki/OpenSSH/Cookbook/Multiplexing#Setting_Up_Multiplexing) so connections after the first are instant by reusing the authorized session.

### Shell completions

`msync` ships dynamic shell completions.
Enable them by adding the relevant snippet to your shell startup file.

```bash
# bash (~/.bashrc)
command -v msync >/dev/null 2>&1 && source <(COMPLETE=bash msync) || true
```

```zsh
# zsh (~/.zshrc)
(( $+commands[msync] )) && source <(COMPLETE=zsh msync) || true
```

```fish
# fish (~/.config/fish/config.fish)
type -q msync && COMPLETE=fish msync | source; or true
```

## Installation

### Nix

Run without installing:

```
nix run github:fabian-thomas/machine-sync
```

Install permanently:

```
nix profile install github:fabian-thomas/machine-sync
```

The flake exposes two packages: `default` (a normal dynamically linked build) and `static` (a fully static binary):

```
nix build github:fabian-thomas/machine-sync#static
```

A dev shell with the Rust toolchain is available via `nix develop`.

### From source

Build it with Cargo:

```
cargo build --release
# or a fully static binary:
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
```

Runtime dependencies: `rsync` and `openssh` on the `PATH`.
For notifications, optionally `notify-send` / a D-Bus notification daemon (Linux desktop), or `wsl-notify-send` (WSL → Windows).

