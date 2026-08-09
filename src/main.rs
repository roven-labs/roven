fn main() {
    if let Err(error) = pmemc::run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
