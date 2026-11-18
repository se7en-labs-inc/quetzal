fn main() {
    if let Err(error) = quetzal_migrate::run_cli() {
        eprintln!("migration failed: {error:#}");
        std::process::exit(1);
    }
}
