//! Reads the request on stdin, prints the result on stdout. Anything else goes to stderr.

use std::io::{self, Read, Write};
use std::process::ExitCode;

use quill_oracle::fetch::Http;
use quill_oracle::{run, Input};

fn main() -> ExitCode {
    let mut request = String::new();
    if let Err(e) = io::stdin().read_to_string(&mut request) {
        eprintln!("cannot read stdin: {e}");
        return ExitCode::FAILURE;
    }
    let input: Input = match serde_json::from_str(&request) {
        Ok(input) => input,
        Err(e) => {
            eprintln!("bad request: {e}");
            return ExitCode::FAILURE;
        }
    };

    let result = run(&input, &Http);
    for line in &result.evidence {
        eprintln!("evidence  {line}");
    }
    for problem in &result.problems {
        eprintln!("problem   {problem}");
    }

    let mut out = io::stdout();
    let printed = serde_json::to_writer(&mut out, &result.output).map_err(|e| e.to_string());
    match printed.and_then(|()| out.flush().map_err(|e| e.to_string())) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("cannot write result: {e}");
            ExitCode::FAILURE
        }
    }
}
