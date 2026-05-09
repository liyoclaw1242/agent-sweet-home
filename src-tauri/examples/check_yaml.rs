fn main() {
    let path = std::env::args().nth(1).expect("usage: check_yaml <path>");
    let txt = std::fs::read_to_string(&path).expect("read");
    match agent_sweet_home_lib::workflow::Workflow::from_yaml(&txt) {
        Ok(wf) => {
            eprintln!("OK: {} roles, {} dispatch rules, {} on_result handler entries",
                wf.roles.len(), wf.dispatch.rules.len(), wf.on_result.len());
        }
        Err(e) => {
            eprintln!("PARSE ERROR: {}", e);
            std::process::exit(1);
        }
    }
}
