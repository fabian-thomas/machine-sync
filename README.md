This repository provides a shell script that uses `lsyncd` to sync a folder live to one or multiple machines.
It uses `rsync`, therefore a ssh connection to the machine is enough.
The script also supports sending out notifications with a list of updated files plus sophisticated ignore mechanism.
For other features and tips refer to the section about [tips and features](#tips-and-features).
The usage is simple:
- `msync start machine1 machine2`: Syncs the current working directory to the home folder of `machine1` and `machine2`.
- `msync` or `msync status`: Lists running instances with their `id`.
- `msync stop <id>`: Stops the sync session with `id`.

The script can be easily installed as a [Nix package](#nix-package) or can be [installed manually](#manual-installation).
The Nix package has the advantage that all dependencies are automatically fetched.

## Tips and Features

### Notifications

`msync` sends out a notification with changed files on each change in the synced directory.

![Image of notification sent out by msync.](imgs/notification.png)

### Ignoring Files
`msync` reads and applies the excludes specified in `.gitignore`.
When there are files that you want to force sync, add a comment line with `# noldignore` to your `.gitignore`.
Every pattern that is listed after this comment, will be excluded when reading the `.gitignore`.
Put any files that you want to exclude, but not via `.gitignore`, in a custom file called `.ldignore`.
`msync` will read and apply the patterns in that file identical to patterns in `.gitignore`.

### SSH Multiplexing
`lsyncd`, the tool that is used for synchronization, spawns `rsync` every time a change needs to be synchronized.
This leads to problems with medium to high latency connections.
I can in general recommend using [SSH Multiplexing](https://en.wikibooks.org/wiki/OpenSSH/Cookbook/Multiplexing#Setting_Up_Multiplexing).
That configuration makes any further connection attempts after the initial connection instant, since the already authorized session is used for the setup.

## Nix package

Test it out without installation:
```
nix run github:fabian-thomas/machine-sync
```

Install it permanently:
```
nix profile install github:fabian-thomas/machine-sync
```

## Manual installation

Dependencies (collected from `default.nix` and `rsync-notify-multiple.nix`):
- coreutils
- findutils
- lsyncd
- ps
- libnotify
- rsync
- openssh

If you move the script to another place, set `LSYNCD_TEMPLATE=/path/to/cloned/repo/template.lua`.
