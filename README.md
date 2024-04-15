This repository provides a shell script that uses `lsyncd` to sync a folder live to one or multiple machines.
It uses `rsync`, therefore a ssh connection to the machine is enough.
The script also supports sending out notifications when each sync completes with a list of updated files.
The usage is simple:
- `msync start machine1 machine2`: Syncs the current working directory to the home folder of `machine1` and `machine2`.
- `msync` or `msync status`: Lists running instances with their `id`.
- `msync stop <id>`: Stops the sync session with `id`.

The script can be either directly installed from this repo or installed as a Nix package.
The latter has the advantage that all dependencies are automatically fetched.

# Nix package

Test it out without installation:
```
nix run github:fabian-thomas/machine-sync
```

Install it permanently:
```
nix profile install github:fabian-thomas/machine-sync
```

# Manual installation

Dependencies (specified in `default.nix` and `rsync-notify-multiple.nix`):
- coreutils
- findutils
- lsyncd
- ps
- libnotify
- rsync
- openssh

If you move the script to another place, set `LSYNCD_TEMPLATE=/path/to/cloned/repo/template.lua`.
