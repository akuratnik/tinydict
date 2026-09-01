# tinydict

Tiny, fast, accurate voice-to-text Linux daemon for GNOME/Hyprland (eg Ubuntu, Fedora, Debian, Arch, Omarchy, etc).

- **No UI**: hotkey to start/stop, paste transcript from clipboard.
- **Does NOT transcribe locally**: streaming cloud API (Speechmatics - has free credits).
- **Optional text cleanup**: uses OpenAI-compatible LLM (slower).

Feels instant. 
Tiny (~20MB RAM when transcribing). 
Accurate, multi-language, supports custom vocab.

```sh
cargo build --release
install -m 755 target/release/tinydict ~/.local/bin/tinydict
tinydict setup
```

Needs PipeWire (`pw-record`), `wl-clipboard`, and a systemd user session.

All settings & keys in `~/.config/tinydict/config.toml`. Reloaded for each recording.

**Hotkey is Ctrl+Super+X**. `tinydict setup` binds it on GNOME and Hyprland. Otherwise bind `tinydict toggle` yourself.

`tinydict toggle` · `tinydict cancel` · `tinydict status`