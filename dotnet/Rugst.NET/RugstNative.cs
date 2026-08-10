using System.Runtime.InteropServices;

namespace Rugst;

/// <summary>
/// rugst.h のエラーコードに対応。
/// </summary>
public enum RugstError : int
{
    /// <summary>正常終了。</summary>
    Ok = 0,
    /// <summary>ヌルポインタが渡されたエラー。</summary>
    NullPointer = 1,
    /// <summary>不正な UTF-8 文字列が渡されたエラー。</summary>
    InvalidUtf8 = 2,
    /// <summary>内部処理で発生したエラー。</summary>
    InternalError = 3
}

/// <summary>
/// rugst_search に渡す検索オプション。rugst.h の RugstSearchOptions と1:1対応。
/// フィールド順序を変更しないこと（ネイティブ側とレイアウトを一致させる必要がある）。
/// </summary>
[StructLayout(LayoutKind.Sequential)]
public struct RugstSearchOptionsNative
{
    /// <summary>取得する上位件数。</summary>
    public uint TopK;
    /// <summary>スコアの半減期(日数)。</summary>
    public float HalfLifeDays;
    /// <summary>検索結果に含める最低スコア。</summary>
    public float MinScore;
    /// <summary>検索対象とする直近候補の件数上限(0以下の場合は全件対象)。</summary>
    public long CandidateWindow;
}

/// <summary>
/// rugst.h の RugstSearchResult に対応する非管理構造体。
/// </summary>
[StructLayout(LayoutKind.Sequential)]
public struct RugstSearchResultNative
{
    /// <summary>検索結果のテキストポインタ (UTF-8)。</summary>
    public IntPtr Text; // char* (UTF-8)
    /// <summary>検索スコア。</summary>
    public float Score;
    /// <summary>作成日時 (Unixタイムスタンプ)。</summary>
    public long CreatedAt;
}

/// <summary>
/// rugst.h の RugstSearchResults に対応する非管理構造体。
/// </summary>
[StructLayout(LayoutKind.Sequential)]
public struct RugstSearchResultsNative
{
    /// <summary>検索結果構造体配列へのポインタ。</summary>
    public IntPtr Results; // RugstSearchResult*
    /// <summary>検索結果の要素数。</summary>
    public nuint Len;
    /// <summary>配列のメモリ割り当て容量。</summary>
    public nuint Capacity;
}

/// <summary>
/// rugst.h の RugstListItem に対応する非管理構造体(事実一覧取得用)。
/// </summary>
[StructLayout(LayoutKind.Sequential)]
public struct RugstListItemNative
{
    /// <summary>レコードの固有ID。</summary>
    public long Id;
    /// <summary>レコードのテキストポインタ (UTF-8)。</summary>
    public IntPtr Text; // char* (UTF-8)
    /// <summary>作成日時 (Unixタイムスタンプ)。</summary>
    public long CreatedAt;
}

/// <summary>
/// rugst.h の RugstListResults に対応する非管理構造体。
/// 使い終わったら必ず rugst_free_list_results に渡して解放すること。
/// </summary>
[StructLayout(LayoutKind.Sequential)]
public struct RugstListResultsNative
{
    /// <summary>リスト項目構造体配列へのポインタ。</summary>
    public IntPtr Items; // RugstListItem*
    /// <summary>リスト項目の要素数。</summary>
    public nuint Len;
    /// <summary>配列のメモリ割り当て容量。</summary>
    public nuint Capacity;
}

/// <summary>
/// rugst.dll の生の P/Invoke シグネチャ。
/// 呼び出しは <see cref="RugstClient"/> を通して行い、このクラスを直接使わないこと。
/// </summary>
internal static class RugstNative
{
    private const string LibName = "rugst.dll";

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern uint rugst_version();

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern RugstSafeHandle rugst_create(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string db_path);

    // RugstSafeHandle.ReleaseHandle からのみ呼ばれる。RugstSafeHandle は
    // 生成直後の(まだ SetHandle されただけの)IntPtr をそのまま解放する必要が
    // あるため、ここだけは SafeHandle ではなく素の IntPtr を受け取る。
    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern void rugst_destroy(IntPtr handle);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern RugstError rugst_remember(
        RugstSafeHandle handle,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string channel_id,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string author_id,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string role,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string content);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern RugstSearchResultsNative rugst_search(
        RugstSafeHandle handle,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string channel_id,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string role,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string query,
        RugstSearchOptionsNative options);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern void rugst_free_search_results(RugstSearchResultsNative results);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern RugstListResultsNative rugst_list(
        RugstSafeHandle handle,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string channel_id,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string role);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern void rugst_free_list_results(RugstListResultsNative results);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern RugstError rugst_delete(RugstSafeHandle handle, long id);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern RugstError rugst_update(
        RugstSafeHandle handle,
        long id,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string content);
}