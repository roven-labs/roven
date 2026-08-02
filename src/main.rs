fn main() {
    if let Err(error) = pmemc::run() {
        if let Some(message) = pmemc::validation_error_message(&error) {
            eprintln!("{message}");
        } else {
            eprintln!("error: {error}");
        }
        std::process::exit(1);
    }
}
