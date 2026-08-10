using Rugst;

using var rugst = RugstClient.Open("memory.db");

rugst.Remember(
    "general",
    "user123",
    "fact",
    "ユーザーはRustが好き"
);

var results = rugst.Search(
    "general",
    "ユーザーの好きなプログラミング言語"
);

foreach (var result in results)
{
    Console.WriteLine($"{result.Score}: {result.Text}");
}