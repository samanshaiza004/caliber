use std::process::ExitCode;

fn main() -> ExitCode {
    match caliber::run(std::env::args_os().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("caliber: {error}");
            ExitCode::FAILURE
        }
    }
}
