use std::io::{Read, Write};

fn main() {
    let mut input = Vec::new();
    // A read error leaves `input` empty; `run` then exits 0 without output.
    let _ = std::io::stdin().read_to_end(&mut input);
    let out = orch_hook::run(&input, &orch_hook::HookEnv::from_env());
    if let Some(s) = out.stdout {
        let mut o = std::io::stdout().lock();
        let _ = writeln!(o, "{s}");
        let _ = o.flush();
    }
    std::process::exit(out.exit);
}
