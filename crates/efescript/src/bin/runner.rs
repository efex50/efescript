//! `efescript-runner`: a slim executable that runs the module embedded in itself.

fn main() {
    match efescript::run_v2::run_embedded() {
        Some(code) => std::process::exit(code),
        None => {
            eprintln!(
                "efescript-runner: no embedded module (create one with: efescript bundle FILE -o OUT)"
            );
            std::process::exit(2);
        }
    }
}
