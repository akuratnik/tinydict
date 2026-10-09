# tinydict

Tiny, fast, accurate voice-to-text daemon for macOS and Linux GNOME/Hyprland (eg Ubuntu, Fedora, Debian, Arch, Omarchy, etc).

- **No UI**: hotkey to start/stop, paste transcript from clipboard.
- **Does NOT transcribe locally**: streaming cloud API (Speechmatics - has free credits).
- **Optional text cleanup**: uses OpenAI-compatible LLM (slower).

Feels instant. 
Tiny (~20MB RAM when transcribing). 
Accurate, multi-language, supports custom vocab.

```sh
cargo build --release
./target/release/tinydict setup   # installs itself to ~/.local/bin
```

Linux needs PipeWire (`pw-record`), `wl-clipboard`, and a systemd user session.

macOS needs `brew install sox`. Setup installs two LaunchAgents: the socket-activated daemon and a tiny hotkey listener. The first recording asks for microphone access; a rebuilt binary may need it re-granted in System Settings → Privacy & Security → Microphone. Notifications come from Script Editor, so allow it in System Settings → Notifications. Logs: `~/.local/share/tinydict/tinydict.log`.

All settings & keys in `~/.config/tinydict/config.toml`. Reloaded for each recording. `tinydict settings` opens that file in the default editor.

Cleanup works with any OpenAI-compatible API. Provider-specific options (e.g. disabling thinking) go in `[cleanup.extra_body]`; the config has OpenRouter and Cerebras examples.

**Hotkey is Ctrl+Super+X** (Ctrl+Cmd+X on macOS). `tinydict setup` binds it on macOS, GNOME and Hyprland. Otherwise bind `tinydict toggle` yourself.

Forgotten recordings auto-stop after 3 min without speech or 30 min total (`silence_stop_secs`, `max_recording_secs`).

`tinydict toggle` · `tinydict cancel` · `tinydict status` · `tinydict settings`