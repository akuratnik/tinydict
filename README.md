# tinydict

Simple voice to text for Linux + GNOME.

- Hotkey to record & stop. 
- Transcribe audio with Speechmatics (has free tier)
- Optional text cleanup with LLM (openAI-compatible).
- Paste transcript from clipboard.

Should feel near-instant without cleanup.

Daemon doesn't persist, launched by next recording.

```sh
cargo build --release
install -m 755 target/release/tinydict ~/.local/bin/tinydict
tinydict setup
```

Needs PipeWire (`pw-record`), `wl-clipboard`, and a systemd user session.

All settings & keys in `~/.config/tinydict/config.toml`. Reloaded for each recording.

**Hotkey** is Ctrl+Super+Space. Change with `tinydict setup --binding '...'` or in GNOME Settings → Keyboard → Custom Shortcuts.

`tinydict toggle` · `tinydict cancel` · `tinydict status`