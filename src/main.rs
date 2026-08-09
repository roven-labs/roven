fn main() {
    if let Err(error) = roven::run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
