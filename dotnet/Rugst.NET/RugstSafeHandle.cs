using System.Runtime.InteropServices;

namespace Rugst;

/// <summary>
/// rugst.dll が返すネイティブハンドル(RugstHandle*)を安全に保持するための SafeHandle。
/// GC/ファイナライズのタイミングに依存する自前ファイナライザと違い、
/// ・二重解放や解放漏れを OS レベルで防止
/// ・P/Invoke呼び出し中にハンドルが横から解放される競合(TOCTOU)を防止
/// ・CriticalFinalizerObject を継承しているため、AppDomainアンロードや
///   プロセス終了時の解放処理がより確実
/// といった利点がある。
/// </summary>
internal sealed class RugstSafeHandle : SafeHandle
{
    /// <summary>
    /// P/Invoke の戻り値としてマーシャラが呼び出す既定コンストラクタ。
    /// rugst_create の戻り値がそのまま SetHandle される。
    /// </summary>
    public RugstSafeHandle() : base(IntPtr.Zero, ownsHandle: true)
    {
    }

    /// <summary>ヌルポインタの場合は無効なハンドルとみなす。</summary>
    public override bool IsInvalid => handle == IntPtr.Zero;

    /// <summary>
    /// ハンドルが不要になったときに一度だけ呼ばれる。
    /// ここで rugst_destroy を呼び出し、ネイティブ側のメモリを解放する。
    /// </summary>
    protected override bool ReleaseHandle()
    {
        RugstNative.rugst_destroy(handle);
        return true;
    }
}