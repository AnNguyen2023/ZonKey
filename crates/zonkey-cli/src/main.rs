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
        _ => {
            println!(
                "Zonkey: architecture and audit phase; use `observe-hook` or `observe-raw` for the Windows spikes."
            );
        }
    }
}
