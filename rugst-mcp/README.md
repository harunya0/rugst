# Rugst MCP

MCP server for Rugst, a local semantic memory store.

Rugst MCP allows MCP-compatible AI assistants and applications to store and retrieve memories using semantic search.

## Features

- remember — Store a memory
- search — Search memories using semantic similarity
- get_recent_history — Retrieve recent conversation history
- ping — Test the MCP server

## Requirements

- Rust 1.88+
- An MCP-compatible client

## Build

cargo build --release

The server binary will be located at:

target/release/rugst-mcp

On Windows:

target/release/rugst-mcp.exe

## Configuration

Rugst MCP uses memory.db by default.

You can specify a database path using the RUGST_DB_PATH environment variable.

### Windows

```bash
{
  "mcpServers": {
    "rugst": {
      "command": "C:\\path\\to\\rugst-mcp.exe",
      "env": {
        "RUGST_DB_PATH": "C:\\path\\to\\memory.db"
      }
    }
  }
}
```
## Tools

### remember

Stores a memory in Rugst.

Arguments:

- channel_id
- author_id
- role
- content

### search

Searches memories using semantic similarity.

Arguments:

- channel_id
- role
- query

### get_recent_history

Retrieves recent conversation history.

Arguments:

- channel_id
- limit

### ping

Returns pong.

## License

MIT