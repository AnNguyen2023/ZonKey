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
        _ => {
            println!(
                "Zonkey: architecture and audit phase; use `observe-hook`, `observe-raw`, or `diagnose` for the Windows spikes."
            );
        }
    }
}
