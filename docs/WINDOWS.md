# croft on Windows

Short version: **build this fork natively and run `croft.exe` in Windows Terminal or WezTerm with
a Nerd Font.** No WSL2 is required. The port keeps the upstream feature set, with the exceptions
listed under [Limitations](#limitations). For what croft does see the [README](../README.md), and
for the full keyboard surface see [KEYBINDINGS.md](KEYBINDINGS.md).

`croft` is based on [vitali87/croft](https://github.com/vitali87/croft); the native Windows support
described here lives in this fork. The port plan and per-area status are in
[Windows_Port.md](../Windows_Port.md).

## Build

Prerequisites: Windows 10/11 x86_64, the MSVC C++ build tools, and a Rust toolchain (1.97+).

```powershell
git clone git@github.com:Zizeo/croft_windows.git
cd croft_windows
cargo install --path . --locked
```

`croft.exe` lands in `%USERPROFILE%\.cargo\bin`. For a local build only, use
`cargo build --release` and run `.\target\release\croft.exe`.

## Pick the terminal first

The terminal decides whether croft's graphics render. croft's visual identity — activity-bar icons,
file-type icons, the hero illustration, and inline image, PDF and spreadsheet previews — is painted
with the iTerm2 OSC 1337 and Kitty graphics protocols, with sixel as a runtime-probed fallback.

| Host terminal | Result |
|---------------|--------|
| **WezTerm** (Windows build) | Full UI: OSC 1337 and Kitty graphics, plus the Kitty keyboard protocol. **Recommended.** |
| **Windows Terminal** 1.22+ | Full UI on recent builds: sixel graphics, probed through DA1 and `CSI 16 t` for cell pixel geometry. |
| Legacy `conhost.exe` (including the default window a bare `powershell.exe` opens) | croft runs, but image-less: Nerd Font glyphs for icons, half-block characters for image previews. croft shows its one-time "switch terminal" nudge on launch. |

Whatever you choose, set a Nerd Font as the terminal font, and make sure truecolor is enabled
(Windows Terminal enables it by default; in WezTerm set `term = "wezterm"` and use a truecolor
color scheme).

## Shells

The embedded terminal discovers these profiles on Windows:

* PowerShell 7+ — `pwsh.exe` (default when installed)
* Windows PowerShell 5.1 — `powershell.exe`
* `cmd.exe`
* Git Bash — `C:\Program Files\Git\bin\bash.exe`
* WSL — `wsl.exe`, for running the Linux build's own commands inside a pane

Shell integration is a generated PowerShell script at
`<config_dir>\shell-integration\powershell\shellIntegration.ps1`. croft launches PowerShell with
`-NoLogo -NoExit -ExecutionPolicy Bypass -File <shim>`, which:

* prepends `CROFT_PANE_BIN` to `PATH`, so `croft view` and friends inside a pane reach the running
  instance;
* sources `$PROFILE.CurrentUserAllHosts` and `$PROFILE.CurrentUserCurrentHost` first, so Oh My Posh,
  Starship and custom prompts keep working (set `CROFT_PS_NO_PROFILE=1` to skip);
* wraps `prompt` to emit `OSC 133;A`/`B`/`D` and `OSC 7` with a `file:///C:/...` working directory,
  and hooks PSReadLine's `Enter` for `OSC 133;C`. That is what gives you prompt navigation, command
  duration, split-inherited working directories and idle-shell detection without POSIX
  `tcgetpgrp`.

Set `CROFT_SHELL_INTEGRATION=0` to launch a plain `-NoLogo` PowerShell instead.

Conversely, if `pwsh` starts with `-File <shim>` and then immediately exits, check the PowerShell
execution policy for that user scope; the bypass flag applies only to croft's own launch.

## Files, saves and processes

Atomic saves retry through transient NTFS sharing violations (`ERROR_ACCESS_DENIED`,
`ERROR_SHARING_VIOLATION`) caused by Defender, the Search Indexer, or a language server holding a
handle. Real read-only protection still fails immediately rather than being retried. Spawned tasks,
run configurations and debug adapters are placed in Win32 Job Objects so stopping them kills the
whole process tree.

## Remote development

`croft remote <host>` and `croft sync-config` work natively through the Windows OpenSSH client
(`ssh.exe`). Because Windows OpenSSH has no `ControlMaster` sockets, every connection is a fresh
SSH session, which makes the first connect and reconnect marginally slower than on macOS or Linux.

## Local IPC and deep links

`croft view <file>`, `croft edit --wait <file>`, `croft hook` (Claude Code pre-tool approvals) and
`croft://` links work through capability-token-authenticated loopback TCP listeners in the croft
cache directory. Register the deep-link handler once with `croft install-link-handler`, which writes
`HKCU\Software\Classes\croft` — user-scope only, no administrator elevation. Leftover descriptor
files from crashed instances are swept on startup by checking whether the owning PID is still
alive.

## Limitations

These features are built on POSIX daemons or OpenSSH `ControlMaster` sockets and are not available
in the native Windows build. Run the upstream Linux build under WSL2 when you need them:

* `croft attach` (persistent, joinable sessions) and `croft ls`
* `croft record` and `croft web`
* `croft pair` and `croft collab` (AI seats and multiplayer)
* `croft devcontainer`
* `croft pane-host` / `croft session-host` internals behind the above

Also note:

* The full internal test suite is not yet ported. `cargo test --test platform` runs on Windows;
  `cargo test --bin croft` still contains Unix-only tests, and Windows CI is not configured.
* No prebuilt Windows binaries are published by this fork; build from source.

## Keybindings

croft's command modifier is `Ctrl` on Windows. `F1` opens the in-app reference.

Inside PowerShell panes croft's shim also remaps two `PSReadLine` keys so its own actions reach
the editor: `Ctrl+e` becomes `EndOfLine` and `Ctrl+u` becomes `BackwardDeleteLine`. Other `Ctrl`
chords may be swallowed by the host terminal or by Windows itself; if a shortcut does not arrive,
check the terminal's own keybinding list first, then remap in croft's keybinding settings. See
[KEYBINDINGS.md](KEYBINDINGS.md).

## Troubleshooting

| Symptom | Likely cause |
|---------|--------------|
| Icons render as `[?]` boxes | The terminal font is not a Nerd Font, or the patched font is not selected for the profile. |
| Icons render as glyphs but previews show half blocks | Running under legacy `conhost.exe`, or a terminal without sixel/OSC 1337 support. Switch to WezTerm or Windows Terminal 1.22+. |
| Images look blurry or wrongly sized | The host ignored `CSI 16 t`; croft falls back to the console font size, which can be off at non-default zoom. |
| A pane does not pick up the current directory | Shell integration did not load; confirm the shell is `pwsh.exe`/`powershell.exe` and that `CROFT_SHELL_INTEGRATION` is not `0`. |
| Tasks fail with syntax errors | The task was written with POSIX quoting for a PowerShell pane. croft quotes for the pane's shell; split POSIX-only tasks into a Git Bash or WSL profile. |
| Save fails with "access denied" on a writable file | Something is holding an exclusive handle (Defender, indexer, LSP). croft retries briefly; retry the save, or exclude the workspace from real-time scanning. |
| `croft view` opens nothing | A firewall prompt for loopback TCP was declined, or a stale descriptor file remains in the croft cache directory. Restart croft and retry. |
