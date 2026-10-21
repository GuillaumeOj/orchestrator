use std::io::{Read, Write};

fn main() {
    let mut input = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut input);
    let out = orch_status::run(&input, &orch_status::StatusEnv::from_env());
    let mut o = std::io::stdout().lock();
    let _ = o.write_all(&out);
    let _ = o.flush();
}
