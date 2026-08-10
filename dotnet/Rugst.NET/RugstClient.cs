using System.Runtime.InteropServices;

namespace Rugst;

/// <summary>
/// 検索結果1件を表す管理側のレコード。
/// </summary>
/// <param name="Text">保存されていた本文</param>
/// <param name="Score">スコア(値が大きいほど関連度が高い想定)</param>
/// <param name="CreatedAtUnix">記録日時(Unixエポック秒)</param>
public readonly record struct RugstSearchHit(string Text, float Score, long CreatedAtUnix);

/// <summary>
/// 事実(fact)レコード1件を表す管理側のレコード。
/// </summary>
public readonly record struct RugstFact(long Id, string Text, long CreatedAtUnix);

/// <summary>
/// rugst_search に渡す検索オプション(管理側)。
/// </summary>
public sealed class RugstSearchOptions
{
    /// <summary>取得する上位件数。</summary>
    public uint TopK { get; init; } = 5;

    /// <summary>スコアの半減期(日数)。新しい記録を優先したい場合に使う。</summary>
    public float HalfLifeDays { get; init; } = 30f;

    /// <summary>このスコア未満の結果は除外する。</summary>
    public float MinScore { get; init; } = 0f;

    /// <summary>
    /// 検索候補の件数上限。0以下を指定すると全件(履歴すべて)が対象になる。
    /// </summary>
    public long CandidateWindow { get; init; } = 0;

    internal RugstSearchOptionsNative ToNative() => new()
    {
        TopK = TopK,
        HalfLifeDays = HalfLifeDays,
        MinScore = MinScore,
        CandidateWindow = CandidateWindow
    };
}

/// <summary>
/// rugst.dll (RAG検索エンジン) を安全に扱うための管理ラッパー。
/// スレッドセーフ性は rugst 側の実装に依存するため保証しない。
/// </summary>
public sealed class RugstClient : IDisposable
{
    private IntPtr _handle;
    private bool _disposed;

    private RugstClient(IntPtr handle)
    {
        _handle = handle;
    }

    /// <summary>
    /// DBファイルを開いて(なければ作成して) RugstClient を生成する。
    /// </summary>
    /// <param name="dbPath">SQLite等のDBファイルパス。rugst側の実装に依存。</param>
    public static RugstClient Open(string dbPath)
    {
        IntPtr handle = RugstNative.rugst_create(dbPath);
        if (handle == IntPtr.Zero)
        {
            throw new InvalidOperationException(
                $"rugst_create に失敗しました (db_path: {dbPath})。パスの権限やディレクトリの存在を確認してください。");
        }
        return new RugstClient(handle);
    }

    /// <summary>ネイティブ側のバージョン番号を取得する。</summary>
    public static uint NativeVersion => RugstNative.rugst_version();

    /// <summary>
    /// 1件の発言を記憶させる。
    /// </summary>
    /// <param name="channelId">チャンネル(会話)を識別するID</param>
    /// <param name="authorId">発言者のID</param>
    /// <param name="role">"user" / "assistant" など</param>
    /// <param name="content">本文</param>
    public void Remember(string channelId, string authorId, string role, string content)
    {
        ThrowIfDisposed();

        RugstError err = RugstNative.rugst_remember(_handle, channelId, authorId, role, content);
        if (err != RugstError.Ok)
        {
            throw new InvalidOperationException($"rugst_remember に失敗しました: {err}");
        }
    }

    /// <summary>
    /// クエリに関連する情報を検索する。
    /// </summary>
    /// <param name="channelId">検索対象のチャンネル</param>
    /// <param name="query">検索クエリ</param>
    /// <param name="options">検索オプション</param>
    /// <param name="role">
    /// 検索対象を絞るrole。既定は"fact"。
    /// role を絞らないと、過去のuser/assistant発言(誤った回答も含む)まで
    /// 検索候補に混ざり、admin側でfactを更新しても古い回答が
    /// 検索結果に残り続けてしまうため、既定でfactのみに限定している。
    /// </param>
    public IReadOnlyList<RugstSearchHit> Search(string channelId, string query, RugstSearchOptions? options = null, string role = "fact")
    {
        ThrowIfDisposed();

        options ??= new RugstSearchOptions();
        RugstSearchResultsNative native = RugstNative.rugst_search(_handle, channelId, role, query, options.ToNative());

        try
        {
            return CopyResults(native);
        }
        finally
        {
            // native.Results が指す配列や各要素の文字列は rugst 側が確保したメモリなので、
            // 必ずこの解放関数経由で返却する(管理側で直接 Marshal.FreeHGlobal してはいけない)。
            RugstNative.rugst_free_search_results(native);
        }
    }

    /// <summary>指定roleのレコードを一覧取得する(既定は"fact")。</summary>
    public IReadOnlyList<RugstFact> ListFacts(string channelId, string role = "fact")
    {
        ThrowIfDisposed();
        RugstListResultsNative native = RugstNative.rugst_list(_handle, channelId, role);
        try
        {
            return CopyListResults(native);
        }
        finally
        {
            RugstNative.rugst_free_list_results(native);
        }
    }

    /// <summary>idを指定して削除する。</summary>
    public void DeleteFact(long id)
    {
        ThrowIfDisposed();
        RugstError err = RugstNative.rugst_delete(_handle, id);
        if (err != RugstError.Ok)
        {
            throw new InvalidOperationException($"rugst_delete に失敗しました: {err}");
        }
    }

    /// <summary>idを指定して本文を更新する。</summary>
    public void UpdateFact(long id, string content)
    {
        ThrowIfDisposed();
        RugstError err = RugstNative.rugst_update(_handle, id, content);
        if (err != RugstError.Ok)
        {
            throw new InvalidOperationException($"rugst_update に失敗しました: {err}");
        }
    }

    private static List<RugstSearchHit> CopyResults(RugstSearchResultsNative native)
    {
        var results = new List<RugstSearchHit>(checked((int)native.Len));
        if (native.Results == IntPtr.Zero || native.Len == 0)
        {
            return results;
        }

        int itemSize = Marshal.SizeOf<RugstSearchResultNative>();
        for (nuint i = 0; i < native.Len; i++)
        {
            IntPtr itemPtr = IntPtr.Add(native.Results, checked((int)i * itemSize));
            RugstSearchResultNative item = Marshal.PtrToStructure<RugstSearchResultNative>(itemPtr);

            string text = item.Text != IntPtr.Zero
                ? Marshal.PtrToStringUTF8(item.Text) ?? string.Empty
                : string.Empty;

            results.Add(new RugstSearchHit(text, item.Score, item.CreatedAt));
        }

        return results;
    }

    private static List<RugstFact> CopyListResults(RugstListResultsNative native)
    {
        var results = new List<RugstFact>(checked((int)native.Len));
        if (native.Items == IntPtr.Zero || native.Len == 0)
        {
            return results;
        }

        int itemSize = Marshal.SizeOf<RugstListItemNative>();
        for (nuint i = 0; i < native.Len; i++)
        {
            IntPtr itemPtr = IntPtr.Add(native.Items, checked((int)i * itemSize));
            RugstListItemNative item = Marshal.PtrToStructure<RugstListItemNative>(itemPtr);

            string text = item.Text != IntPtr.Zero
                ? Marshal.PtrToStringUTF8(item.Text) ?? string.Empty
                : string.Empty;

            results.Add(new RugstFact(item.Id, text, item.CreatedAt));
        }

        return results;
    }

    private void ThrowIfDisposed()
    {
        if (_disposed)
        {
            throw new ObjectDisposedException(nameof(RugstClient));
        }
    }

    /// <summary>
    /// <see cref="RugstClient"/> で使用されているアンマネージド リソースを解放します。
    /// </summary>
    public void Dispose()
    {
        if (_disposed)
        {
            return;
        }

        if (_handle != IntPtr.Zero)
        {
            RugstNative.rugst_destroy(_handle);
            _handle = IntPtr.Zero;
        }

        _disposed = true;
        GC.SuppressFinalize(this);
    }

    /// <summary>
    /// <see cref="RugstClient"/> のインスタンスがガベージ コレクションによって回収される際に、ネイティブ リソースを解放します。
    /// </summary>
    ~RugstClient()
    {
        // ファイナライザからはネイティブ解放のみ行う(マネージドオブジェクトには触れない)。
        if (_handle != IntPtr.Zero)
        {
            RugstNative.rugst_destroy(_handle);
            _handle = IntPtr.Zero;
        }
    }
}