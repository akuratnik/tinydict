mod audio;
mod cleanup;
mod cli;
mod config;
mod daemon;
#[cfg(target_os = "macos")]
mod hotkey;
mod output;
mod setup;
mod speechmatics;
mod tray;

fn main() {
    let mut args = std::env::args().skip(1);
    let cmd = args.next();
    let result = match cmd.as_deref() {
        Some("daemon") => daemon::run(),
        #[cfg(target_os = "macos")]
        Some("hotkey") => hotkey::run(),
        Some("toggle") => cli::send("toggle"),
        Some("cancel") => cli::send("cancel"),
        Some("status") => cli::send("status"),
        Some("settings") => config::open_settings(),
        Some("setup") => {
            let mut binding = None;
            while let Some(arg) = args.next() {
                if arg == "--binding" {
                    binding = args.next();
                } else {
                    eprintln!("unknown setup flag: {arg}");
                    std::process::exit(2);
                }
            }
            setup::run(binding.as_deref())
        }
        Some("-h" | "--help" | "help") | None => {
            print_usage();
            Ok(())
        }
        Some(other) => {
            eprintln!("unknown command: {other}");
            print_usage();
            std::process::exit(2);
        }
    };
    if let Err(err) = result {
        eprintln!("tinydict: {err:#}");
        std::process::exit(1);
    }
}

fn print_usage() {
    eprint!(
        "\
tinydict — hotkey transcription

  tinydict setup [--binding KEY]   install units, dirs, and hotkey
  tinydict settings                open config.toml in the default editor
  tinydict toggle                  start/stop a recording
  tinydict cancel                  abort the current recording
  tinydict status                  print daemon state
  tinydict daemon                  run the daemon (socket activated)
"
    );
    #[cfg(target_os = "macos")]
    eprintln!("  tinydict hotkey                  listen for Ctrl+Cmd+X (launchd agent)");
}
