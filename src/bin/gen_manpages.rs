//! Man-page generator for gatemini.
//!
//! Run from the repository root to emit roff(5) man pages into
//! `$CARGO_MANIFEST_DIR/manpages/`:
//!
//!     cargo run -p gatemini --bin gen_manpages
//!
//! Or pass an explicit output directory:
//!
//!     cargo run -p gatemini --bin gen_manpages -- --output-dir ./manpages
//!
//! Each invocation writes:
//!   - gatemini.1          — root CLI
//!   - gatemini-serve.1    — serve subcommand
//!   - gatemini-status.1   — status subcommand
//!   - gatemini-stop.1     — stop subcommand
//!   - gatemini-restart.1  — restart subcommand
//!   - gatemini-upgrade.1  — upgrade subcommand
//!   - gatemini-doctor.1   — doctor subcommand

use std::io::Write as IoWrite;
use std::path::PathBuf;

use gatemini::cli::{Cli, Command as CliCommand, prismgate_home};

// ---------------------------------------------------------------------------
// Per-subcommand command builders (reconstruct Clap commands with defaults,
// without relying on unstable Clap internals).
// ---------------------------------------------------------------------------

fn make_serve() -> clap::Command {
    CliCommand::Serve {
        socket: None,
        promote_to: None,
        old_pid: None,
    }
    .into()
}

fn make_upgrade() -> clap::Command {
    CliCommand::Upgrade {
        timeout: std::time::Duration::from_secs(60),
    }
    .into()
}

fn make_status() -> clap::Command {
    CliCommand::Status.into()
}

fn make_stop() -> clap::Command {
    CliCommand::Stop.into()
}

fn make_restart() -> clap::Command {
    CliCommand::Restart.into()
}

fn make_doctor() -> clap::Command {
    CliCommand::Doctor.into()
}

// ---------------------------------------------------------------------------
// Rendering helpers.
// ---------------------------------------------------------------------------

fn render_to_bytes(cmd: &mut clap::Command) -> std::io::Result<Vec<u8>> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    clap_mangen::render(cmd, &mut cursor)?;
    Ok(cursor.into_inner())
}

fn root_command() -> clap::Command {
    <Cli as clap::CommandFactory>::command()
}

// ---------------------------------------------------------------------------
// Page manifest.
// ---------------------------------------------------------------------------

const SUBCOMMANDS: &[(&str, fn() -> clap::Command)] = &[
    ("gatemini-serve", make_serve),
    ("gatemini-status", make_status),
    ("gatemini-stop", make_stop),
    ("gatemini-restart", make_restart),
    ("gatemini-upgrade", make_upgrade),
    ("gatemini-doctor", make_doctor),
];

// ---------------------------------------------------------------------------
// Entrypoint (called from main).
// ---------------------------------------------------------------------------

pub fn emit_manpages(output_dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(output_dir)?;

    // Root man page: gatemini(1)
    let mut root = root_command();
    let roff = render_to_bytes(&mut root)?;
    let path = output_dir.join("gatemini.1");
    std::fs::write(&path, &roff)?;
    println!("wrote {}", path.display());

    // Subcommand pages
    for (name, maker) in SUBCOMMANDS {
        let mut cmd = maker();
        let roff = render_to_bytes(&mut cmd)?;
        let path = output_dir.join(format!("{name}.1"));
        std::fs::write(&path, &roff)?;
        println!("wrote {}", path.display());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// CLI.
// ---------------------------------------------------------------------------

/// Override output directory via `--output-dir`.  Defaults to
/// `$CARGO_MANIFEST_DIR/manpages`.
#[derive(clap::Parser)]
struct Opts {
    #[arg(long)]
    output_dir: Option<PathBuf>,
}

impl Opts {
    fn output_dir(&self) -> PathBuf {
        self.output_dir.clone().unwrap_or_else(|| {
            std::env::var("CARGO_MANIFEST_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| prismgate_home())
                .join("manpages")
        })
    }
}

fn main() -> std::io::Result<()> {
    let opts = Opts::parse();
    emit_manpages(&opts.output_dir())
}
