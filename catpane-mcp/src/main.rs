use catpane_mcp::server;

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to create Tokio runtime");

    rt.block_on(server::run_stdio_server(rt.handle().clone()))
        .expect("MCP server exited with error");
}
