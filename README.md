# Rugst

Local semantic memory store for Rust.

Rugst is a lightweight local memory library that stores conversation history
in SQLite, generates embeddings locally with FastEmbed, and retrieves
semantically similar memories with time-based decay.

[Get NuGet here.](https://www.nuget.org/packages/Rugst)
[The crate is here.](https://crates.io/crates/rugst)

## Features

- Local vector embedding with FastEmbed (multilingual, incl. Japanese)
- SQLite-based persistent memory
- Semantic similarity search
- Hybrid search (vector similarity + FTS5/BM25 keyword search, merged with
  Reciprocal Rank Fusion)
- Time-based memory decay
- Configurable search options
- Fact management API (list / update / delete stored facts by id)
- C-compatible FFI API
- No external vector database required

## Architecture

```
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
```

## Installation

Add Rugst to your Cargo.toml:

```toml
[dependencies]
rugst = "0.2.7"
```

## Rust API

### Create a memory store

```rust
use rugst::Rugst;

let rugst = Rugst::new("memory.db")?;
```

### Store a message

```rust
rugst.remember(
    "general",
    "user",
    "user",
    "I'm studying Rust.",
)?;
```

### Search memories

```rust
use rugst::SearchOptions;

let options = SearchOptions {
    top_k: 5,
    half_life_days: 30.0,
    min_score: 0.3,
    ..Default::default()
};

let results = rugst.search(
    "general",
    "fact",
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
```

### Hybrid search (vector + keyword)

Set `enable_fts: true` to combine semantic similarity with FTS5/BM25 keyword
matching. The two rankings are merged with Reciprocal Rank Fusion (RRF) and
normalized back to a 0–1 scale, so `min_score` still works the same way as
in vector-only search.

```rust
let options = SearchOptions {
    top_k: 5,
    half_life_days: 30.0,
    min_score: 0.3,
    enable_fts: true,
    rrf_k: 60,       // RRF k parameter (higher = flatter rank influence)
    fts_weight: 1.0, // weight of the keyword match relative to vector similarity
    ..Default::default()
};

let results = rugst.search("general", "fact", "opening hours", &options)?;
```

### Manage stored facts

Facts (records saved with `role = "fact"`) can be listed, edited, or removed
by id — useful for building an admin UI on top of Rugst.

```rust
// List all facts in a channel
let facts = rugst.list_by_role("general", "fact")?;
for (id, text, created_at) in facts {
    println!("#{id}: {text} ({created_at})");
}

// Update a fact's content (embedding is recomputed automatically)
rugst.update(42, "Updated fact content")?;

// Delete a fact
rugst.delete(42)?;
```

### Get recent history

Fetches the channel's recent messages in chronological (oldest-first) order,
intended for building prompts to send to an AI.

```rust
let history = rugst.get_recent_history("general", 20)?;

for (role, content) in history {
    println!("{role}: {content}");
}
```

## Search options

| Option | Description |
|---|---|
| top_k | Maximum number of results |
| half_life_days | Half-life used for time decay |
| min_score | Minimum score required for a result (0–1 scale, applies to both vector-only and hybrid search) |
| candidate_window | Limit search to the N most recent candidates. `None` (or ≤0 over FFI) searches the whole channel |
| enable_fts | Enable hybrid search (vector + FTS5/BM25 keyword search via RRF). Defaults to vector-only |
| rrf_k | RRF k parameter used when `enable_fts` is true. Defaults to 60 |
| fts_weight | Weight applied to the keyword match score when `enable_fts` is true. Defaults to 1.0 |

The final search score combines semantic similarity (or the RRF-merged score,
in hybrid mode) with time decay.

Older memories gradually become less relevant, while semantically similar
recent memories receive a higher score.

## C / FFI API

Rugst also provides a C-compatible API for use from other languages.

### Create

```c
RugstHandle *handle = rugst_create("memory.db");
```

### Store a memory

```c
RugstError result = rugst_remember(
    handle,
    "general",
    "user",
    "user",
    "Hello, Rust!"
);
```

### Search

```c
RugstSearchOptions options = {
    .top_k = 5,
    .half_life_days = 30.0f,
    .min_score = 0.3f,
    .candidate_window = 0,   // <= 0 means "search all messages in the channel"
    .enable_fts = 0,         // 1 to enable hybrid vector + FTS5/BM25 search
    .rrf_k = 0,              // <= 0 uses the default (60)
    .fts_weight = 0.0f       // <= 0 uses the default (1.0)
};

RugstSearchResults results =
    rugst_search(handle, "general", "fact", "Rust", options);
```

Note that `role` (`"fact"` here) filters which records are searched — pass
the same `role` you used with `rugst_remember`.

### Get recent history

```c
RugstHistoryResults history =
    rugst_get_recent_history(handle, "general", 20);
```

### Manage stored facts

```c
// List facts
RugstListResults facts = rugst_list(handle, "general", "fact");
// ... free with rugst_free_list_results(facts);

// Update a fact (embedding is recomputed automatically)
RugstError update_result = rugst_update(handle, id, "Updated fact content");

// Delete a fact
RugstError delete_result = rugst_delete(handle, id);
```

### Free search results

Search results allocated by Rugst must be released with:

```c
rugst_free_search_results(results);
```

### Free list results

```c
rugst_free_list_results(facts);
```

### Free history results

History results allocated by Rugst must be released with:

```c
rugst_free_history_results(history);
```

### Destroy

When the handle is no longer needed:

```c
rugst_destroy(handle);
```

## Error handling

The FFI API uses `RugstError`:

```c
typedef enum {
    RUGST_OK = 0,
    RUGST_NULL_POINTER = 1,
    RUGST_INVALID_UTF8 = 2,
    RUGST_INTERNAL_ERROR = 3
} RugstError;
```

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

// Search memories (hybrid search + fact management options shown)
var options = new RugstSearchOptions
{
    TopK = 5,
    HalfLifeDays = 30f,
    MinScore = 0.3f,
    EnableFts = true, // combine vector similarity with FTS5/BM25 keyword search
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

// Get recent history (for building an AI prompt)
var history = client.GetRecentHistory(channelId: "general", limit: 20);

foreach (var entry in history)
{
    Console.WriteLine($"{entry.Role}: {entry.Content}");
}

// Manage stored facts (role="fact" records), e.g. for an admin UI
var facts = client.ListFacts(channelId: "general");
client.UpdateFact(id: facts[0].Id, content: "Updated fact content");
client.DeleteFact(id: facts[0].Id);
```

## Examples

A basic Rust example is available at:

```
examples/RustExample.rs
```

Run it with:

```bash
cargo run --example RustExample
```

## First run

FastEmbed may download the embedding model when Rugst is initialized for
the first time.

An internet connection may therefore be required during the initial setup.

## License

MIT License