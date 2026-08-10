using System.Runtime.InteropServices;

namespace Rugst;

/// <summary>
/// rugst.h のエラーコードに対応。
/// </summary>
public enum RugstError : int
{
    Ok = 0,
    NullPointer = 1,
    InvalidUtf8 = 2,
    InternalError = 3
}

/// <summary>
/// rugst_search に渡す検索オプション。rugst.h の RugstSearchOptions と1:1対応。
/// フィールド順序を変更しないこと（ネイティブ側とレイアウトを一致させる必要がある）。
/// </summary>
[StructLayout(LayoutKind.Sequential)]
public struct RugstSearchOptionsNative
{
    public uint TopK;
    public float HalfLifeDays;
    public float MinScore;
    public long CandidateWindow;
}

/// <summary>
/// rugst.h の RugstSearchResult に対応する非管理構造体。
/// </summary>
[StructLayout(LayoutKind.Sequential)]
public struct RugstSearchResultNative
{
    public IntPtr Text; // char* (UTF-8)
    public float Score;
    public long CreatedAt;
}

/// <summary>
/// rugst.h の RugstSearchResults に対応する非管理構造体。
/// </summary>
[StructLayout(LayoutKind.Sequential)]
public struct RugstSearchResultsNative
{
    public IntPtr Results; // RugstSearchResult*
    public nuint Len;
    public nuint Capacity;
}

/// <summary>
/// rugst.h の RugstListItem に対応する非管理構造体(事実一覧取得用)。
/// </summary>
[StructLayout(LayoutKind.Sequential)]
public struct RugstListItemNative
{
    public long Id;
    public IntPtr Text; // char* (UTF-8)
    public long CreatedAt;
}

/// <summary>
/// rugst.h の RugstListResults に対応する非管理構造体。
/// 使い終わったら必ず rugst_free_list_results に渡して解放すること。
/// </summary>
[StructLayout(LayoutKind.Sequential)]
public struct RugstListResultsNative
{
    public IntPtr Items; // RugstListItem*
    public nuint Len;
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
    public static extern IntPtr rugst_create(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string db_path);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern void rugst_destroy(IntPtr handle);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern RugstError rugst_remember(
        IntPtr handle,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string channel_id,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string author_id,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string role,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string content);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern RugstSearchResultsNative rugst_search(
        IntPtr handle,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string channel_id,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string role,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string query,
        RugstSearchOptionsNative options);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern void rugst_free_search_results(RugstSearchResultsNative results);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern RugstListResultsNative rugst_list(
        IntPtr handle,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string channel_id,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string role);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern void rugst_free_list_results(RugstListResultsNative results);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern RugstError rugst_delete(IntPtr handle, long id);

    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
    public static extern RugstError rugst_update(
        IntPtr handle,
        long id,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string content);
}