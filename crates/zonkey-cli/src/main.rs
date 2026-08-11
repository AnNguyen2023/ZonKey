fn main() {
    if std::env::args().nth(1).as_deref() == Some("observe") {
        if let Err(error) = zonkey_win::run_observe() {
            eprintln!("ZonKey observe failed: {error}");
            std::process::exit(1);
        }
    } else {
        println!("Zonkey: architecture and audit phase; use `observe` for the Windows spike.");
    }
}
