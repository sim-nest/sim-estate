// conformance: model refusal and controller-loss cases preserve the estate dispatch boundary.
// bin-boot-exempt: acceptance harness for typed estate organs, not a product CLI.

mod driver;
mod support;

fn main() {
    if let Err(error) = driver::run(std::env::args().skip(1)) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
