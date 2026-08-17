fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("observe" | "observe-hook") => {
            if let Err(error) = zonkey_win::run_observe() {
                eprintln!("ZonKey observe failed: {error}");
                std::process::exit(1);
            }
        }
        Some("observe-raw") => {
            if let Err(error) = zonkey_win::run_observe_raw() {
                eprintln!("ZonKey raw observe failed: {error}");
                std::process::exit(1);
            }
        }
        Some("diagnose" | "observe-decisions") => {
            let show_token = std::env::args().any(|arg| arg == "--show-token");
            let processor = zonkey_service::DiagnosticDecisionProcessor::new(show_token);
            if let Err(error) = zonkey_win::run_observe_with_processor(processor) {
                eprintln!("ZonKey diagnostic observe failed: {error}");
                std::process::exit(1);
            }
        }
        Some("probe-uia-edit") => {
            let arguments = std::env::args().skip(2).collect::<Vec<_>>();
            let (expected, delay_ms) = match parse_probe_args(&arguments) {
                Ok(value) => value,
                Err(message) => {
                    eprintln!(
                        "usage: probe-uia-edit --expected <token> [--delay-ms <0..=30000>] ({message})"
                    );
                    std::process::exit(2);
                }
            };
            if delay_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
            }
            run_uia_probe(&expected);
        }
        Some("serve-host-validation") => {
            let arguments = std::env::args().skip(2).collect::<Vec<_>>();
            let (pipe_name, max_seconds, handoff_token) = match parse_serve_args(&arguments) {
                Ok(value) => value,
                Err(message) => {
                    eprintln!(
                        "usage: serve-host-validation --pipe <name> [--max-seconds <n>] [--handoff-token <letters>] ({message})"
                    );
                    std::process::exit(2);
                }
            };
            if let Err(error) = zonkey_win::run_serve_host_validation(
                &pipe_name,
                max_seconds,
                handoff_token.as_deref(),
            ) {
                eprintln!("Zonkey host-validation endpoint failed: {error}");
                std::process::exit(1);
            }
        }
        Some("handoff-live") => {
            let arguments = std::env::args().skip(2).collect::<Vec<_>>();
            let (pipe_name, _, _) = match parse_serve_args(&arguments) {
                Ok(value) => value,
                Err(message) => {
                    eprintln!("usage: handoff-live --pipe <name> ({message})");
                    std::process::exit(2);
                }
            };
            if let Err(error) = zonkey_win::run_handoff_live(&pipe_name) {
                eprintln!("Zonkey live handoff endpoint failed: {error}");
                std::process::exit(1);
            }
        }
        Some("recovery") => {
            let arguments = std::env::args().skip(2).collect::<Vec<_>>();
            if let Err(message) = run_recovery_command(&arguments) {
                eprintln!(
                    "usage: recovery --pipe <name> <list|block uri expected replacement start end|reconcile uri expected epoch live|ack uri expected epoch> ({message})"
                );
                std::process::exit(2);
            }
        }
        _ => {
            println!(
                "Zonkey: architecture and audit phase; use `observe-hook`, `observe-raw`, or `diagnose` for the Windows spikes."
            );
        }
    }
}

fn parse_probe_args(arguments: &[String]) -> Result<(String, u64), &'static str> {
    if arguments.len() < 2 || arguments[0] != "--expected" {
        return Err("expected token is required");
    }
    let expected = arguments[1].clone();
    let delay_ms = match arguments {
        [_, _] => 0,
        [_, _, flag, value] if flag == "--delay-ms" => value
            .parse::<u64>()
            .map_err(|_| "delay must be an integer")?,
        _ => return Err("unsupported option"),
    };
    if delay_ms > 30_000 {
        return Err("delay exceeds 30000ms");
    }
    Ok((expected, delay_ms))
}

/// Operator recovery tooling (M3D-28): connects to a running validation
/// endpoint and performs one session-bound recovery command. Query/state
/// only; never mutates documents.
fn run_recovery_command(arguments: &[String]) -> Result<(), String> {
    let mut pipe_name: Option<String> = None;
    let mut rest: Vec<&str> = Vec::new();
    let mut index = 0;
    while index < arguments.len() {
        if arguments[index] == "--pipe" && index + 1 < arguments.len() {
            pipe_name = Some(arguments[index + 1].clone());
            index += 2;
        } else {
            rest.push(arguments[index].as_str());
            index += 1;
        }
    }
    let pipe_name = pipe_name.ok_or("pipe name is required")?;
    if !pipe_name.starts_with(r"\\.\pipe\") {
        return Err("pipe name must be a local named pipe path".to_owned());
    }
    let command = match rest.as_slice() {
        ["list"] => "LIST".to_owned(),
        ["block", uri, expected, replacement, start, end] => {
            if start.parse::<usize>().is_err() || end.parse::<usize>().is_err() {
                return Err("range must be integers".to_owned());
            }
            format!("BLOCK|{uri}|{expected}|{replacement}|{start}|{end}")
        }
        ["reconcile", uri, expected, epoch, live] => {
            if epoch.parse::<u64>().is_err() {
                return Err("document epoch must be an integer".to_owned());
            }
            format!("RECONCILE|{uri}|{expected}|{epoch}|{live}")
        }
        ["ack", uri, expected, epoch] => {
            if epoch.parse::<u64>().is_err() {
                return Err("document epoch must be an integer".to_owned());
            }
            format!("ACK|{uri}|{expected}|{epoch}")
        }
        _ => return Err("unknown recovery command".to_owned()),
    };
    zonkey_win::pipe_transport::PipeClient::connect(&pipe_name, std::time::Duration::from_secs(8))
        .and_then(|mut client| client.recovery_command(&command, std::time::Duration::from_secs(8)))
        .map(|answer| {
            println!("recovery_answer={answer}");
        })
        .map_err(|error| format!("recovery command failed: {error:?}"))
}

fn parse_serve_args(
    arguments: &[String],
) -> Result<(String, Option<u64>, Option<String>), &'static str> {
    let mut pipe_name: Option<String> = None;
    let mut max_seconds: Option<u64> = None;
    let mut handoff_token: Option<String> = None;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--pipe" if index + 1 < arguments.len() => {
                pipe_name = Some(arguments[index + 1].clone());
                index += 2;
            }
            "--max-seconds" if index + 1 < arguments.len() => {
                max_seconds = Some(
                    arguments[index + 1]
                        .parse::<u64>()
                        .map_err(|_| "max-seconds must be an integer")?,
                );
                index += 2;
            }
            "--handoff-token" if index + 1 < arguments.len() => {
                let token = arguments[index + 1].clone();
                if token.is_empty() || !token.chars().all(|c| c.is_ascii_alphabetic()) {
                    return Err("handoff token must be non-empty ASCII letters");
                }
                handoff_token = Some(token);
                index += 2;
            }
            _ => return Err("unsupported option"),
        }
    }
    let pipe_name = pipe_name.ok_or("pipe name is required")?;
    if !pipe_name.starts_with(r"\\.\pipe\") {
        return Err("pipe name must be a local named pipe path");
    }
    Ok((pipe_name, max_seconds, handoff_token))
}

#[cfg(windows)]
fn run_uia_probe(expected: &str) {
    match zonkey_win::probe_focused_edit(expected) {
        Ok(evidence) => {
            println!("uia_probe=pass");
            println!("control=standard-edit");
            println!("selection=empty");
            println!(
                "text_match={}",
                if evidence.exact_preceding_text {
                    "yes"
                } else {
                    "no"
                }
            );
            println!("candidate_units={}", evidence.candidate_text_units);
        }
        Err(reason) => {
            println!("uia_probe=reject reason={reason:?}");
        }
    }
}

#[cfg(not(windows))]
fn run_uia_probe(_: &str) {
    println!("uia_probe=reject reason=windows-only");
}

#[cfg(test)]
mod tests {
    #[test]
    fn probe_expected_argument_is_parsed_after_subcommand() {
        let args = vec!["--expected".to_owned(), "resume".to_owned()];
        assert_eq!(super::parse_probe_args(&args), Ok(("resume".to_owned(), 0)));
    }

    #[test]
    fn probe_delay_argument_is_bounded_and_optional() {
        let args = vec![
            "--expected".to_owned(),
            "resume".to_owned(),
            "--delay-ms".to_owned(),
            "3000".to_owned(),
        ];
        assert_eq!(
            super::parse_probe_args(&args),
            Ok(("resume".to_owned(), 3000))
        );
        let too_long = vec![
            "--expected".to_owned(),
            "resume".to_owned(),
            "--delay-ms".to_owned(),
            "30001".to_owned(),
        ];
        assert_eq!(
            super::parse_probe_args(&too_long),
            Err("delay exceeds 30000ms")
        );
    }
}
