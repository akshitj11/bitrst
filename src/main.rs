//! `bitrst` binary entry point.

use std::process::ExitCode;

fn main() -> ExitCode {
    match bitrst::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
