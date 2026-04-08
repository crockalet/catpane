run:
    cargo run

run-release:
    cargo run -r

rerun:
    cargo clean -p catpane-cli -p catpane-ui -p catpane-core -p catpane-mcp
    cargo run

rerun-release:
    cargo clean --release -p catpane-cli -p catpane-ui -p catpane-core -p catpane-mcp
    cargo run -r

mcp:
    cargo run -p catpane-cli -- mcp
