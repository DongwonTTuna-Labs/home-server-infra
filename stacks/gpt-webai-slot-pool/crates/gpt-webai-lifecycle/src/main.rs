use std::process::ExitCode;

use gpt_webai_lifecycle::cli;

fn main() -> ExitCode {
    match cli::run_os(std::env::args_os().skip(1).collect()) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(error.exit_code())
        }
    }
}
