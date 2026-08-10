# Rugst

Local semantic memory store for Rust.

Rugst is a lightweight local memory library that stores conversation history
in SQLite, generates embeddings locally with FastEmbed, and retrieves
semantically similar memories with time-based decay.

[Get NuGet here.](https://www.nuget.org/packages/Rugst)

## Features

- Local vector embedding with FastEmbed
- SQLite-based persistent memory
- Semantic similarity search
- Time-based memory decay
- Configurable search options
- C-compatible FFI API
- No external vector database required

## Architecture

    Application
        │
        ▼
      Rugst
        ├── LocalEmbedding
        │      └── FastEmbed
        │
        ├── HistoryStore
        │      └── SQLite
        │
        └── Semantic Search
               ├── Cosine similarity
               └── Time decay

## Installation

Add Rugst to your Cargo.toml:

    [dependencies]
    rugst = "0.2.2"

## Rust API

### Create a memory store

    use rugst::Rugst;

    let rugst = Rugst::new("memory.db")?;

### Store a message

    rugst.remember(
        "general",
        "user",
        "user",
        "I'm studying Rust.",
    )?;

### Search memories

    use rugst::SearchOptions;

    let options = SearchOptions {
        top_k: 5,
        half_life_days: 30.0,
        min_score: 0.3,
    };

    let results = rugst.search(
        "general",
        "What I talked about regarding Rust",
        &options,
    )?;

    for result in results {
        println!(
            "[{:.4}] {} ({})",
            result.score,
            result.text,
            result.created_at
        );
    }

## Search options

| Option | Description |
|---|---|
| top_k | Maximum number of results |
| half_life_days | Half-life used for time decay |
| min_score | Minimum score required for a result |

The final search score combines semantic similarity with time decay.

Older memories gradually become less relevant, while semantically similar
recent memories receive a higher score.

## C / FFI API

Rugst also provides a C-compatible API for use from other languages.

## .NET / C# API

Rugst is also available as a [.NET NuGet Package](https://www.nuget.org/packages/Rugst).

### Installation (.NET)

Install via .NET CLI:
```bash
dotnet add package Rugst
```

### Usage (C#)
```csharp
using Rugst;

// Open or create a local memory store
using var client = RugstClient.Open("memory.db");

// Store a message
client.Remember(
    channelId: "general",
    authorId: "user1",
    role: "user",
    content: "I'm learning about Rust and .NET bindings."
);

// Search memories
var options = new RugstSearchOptions
{
    TopK = 5,
    HalfLifeDays = 30f,
    MinScore = 0.3f
};

var results = client.Search(
    channelId: "general",
    query: "What I am studying",
    options: options
);

foreach (var hit in results)
{
    Console.WriteLine($"[{hit.Score:F4}] {hit.Text} (Unix: {hit.CreatedAtUnix})");
}
```

### Create

    RugstHandle *handle = rugst_create("memory.db");

### Store a memory

    RugstError result = rugst_remember(
        handle,
        "general",
        "user",
        "user",
        "Hello, Rust!"
    );

### Search

    RugstSearchOptions options = {
        .top_k = 5,
        .half_life_days = 30.0f,
        .min_score = 0.3f
    };

    RugstSearchResults results =
        rugst_search(handle, "general", "Rust", options);

### Free search results

Search results allocated by Rugst must be released with:

    rugst_free_search_results(results);

### Destroy

When the handle is no longer needed:

    rugst_destroy(handle);

## Error handling

The FFI API uses RugstError:

    typedef enum {
        RUGST_OK = 0,
        RUGST_NULL_POINTER = 1,
        RUGST_INVALID_UTF8 = 2,
        RUGST_INTERNAL_ERROR = 3
    } RugstError;

## Examples

A basic Rust example is available at:

    examples/basic.rs

Run it with:

    cargo run --example basic

## First run

FastEmbed may download the embedding model when Rugst is initialized for
the first time.

An internet connection may therefore be required during the initial setup.

## License

MIT license