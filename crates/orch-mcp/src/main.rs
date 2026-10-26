fn main() {
    let env = orch_mcp::McpEnv::from_env();
    orch_mcp::serve(std::io::stdin().lock(), std::io::stdout().lock(), &env);
}
