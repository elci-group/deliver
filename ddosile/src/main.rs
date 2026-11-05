use ddosile::compile_policy;
use std::{env, fs, process::ExitCode};

fn main() -> ExitCode {
    let args: Vec<_> = env::args().collect();
    if args.len() != 3 || args[1] != "check" {
        eprintln!("usage: ddosile check <policy.toml>");
        return ExitCode::from(2);
    }
    let source = match fs::read_to_string(&args[2]) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("cannot read {}: {error}", args[2]);
            return ExitCode::FAILURE;
        }
    };
    match compile_policy(&source) {
        Ok(policy) => {
            print!("{}", policy.canonical_text());
            ExitCode::SUCCESS
        }
        Err(errors) => {
            for error in errors {
                eprintln!("policy error: {}", error.0);
            }
            ExitCode::FAILURE
        }
    }
}
