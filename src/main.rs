mod cli;
mod commands;
mod config;
mod heartbeat;
mod ignoreset;
mod notify;
mod paths;
mod registry;
mod spec;
mod target;
mod transport;
mod ui;
mod worker;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command};

fn main() {
    if let Err(e) = run() {
        eprintln!("msync: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    // Dynamic shell completion: when invoked by the shell completion hook this
    // handles the request and exits; otherwise it is a no-op.
    clap_complete::CompleteEnv::with_factory(<Cli as clap::CommandFactory>::command).complete();

    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Status(cli::StatusArgs {
        all: false,
        json: false,
    })) {
        Command::Status(a) => commands::status(&a),
        Command::Start(a) => commands::start(&a),
        Command::Stop(a) => commands::stop(&a),
        Command::Pause(a) => commands::pause(&a),
        Command::Resume(a) => commands::resume(&a),
        Command::Restart(a) => commands::restart(&a),
        Command::Logs(a) => commands::logs(&a),
    }
}
