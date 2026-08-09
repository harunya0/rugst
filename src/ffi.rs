use crate::Rugst;
use std::ffi::{c_char, CStr};
use std::panic::catch_unwind;
use std::sync::Mutex;

impl From<&RugstSearchOptions> for crate::search::SearchOptions {
    fn from(value: &RugstSearchOptions) -> Self {
        Self {
            top_k: value.top_k as usize,
            half_life_days: value.half_life_days,
            min_score: value.min_score,
            // C側にOption<T>は無いのでセンチネル方式にする:
            // 0以下 = 制限なし(全件対象)、正の値 = 直近n件に限定
            candidate_window: if value.candidate_window > 0 {
                Some(value.candidate_window)
            } else {
                None
            },
        }
    }
}

//エラーの定義
#[repr(C)]
pub enum RugstError {
    Ok = 0,
    NullPointer = 1,
    InvalidUtf8 = 2,
    InternalError = 3,
}
//ダミー
//複数スレッドから同じハンドルを叩かれても内部状態が壊れないよう、
//Mutexで排他制御する(以前はここに排他制御が一切なく、マルチスレッドから
//呼ばれるとUBになっていた)
#[repr(C)]
pub struct RugstHandle {
    inner: Mutex<Rugst>,
}
//検索オプション
#[repr(C)]
pub struct RugstSearchOptions {
    pub top_k: u32,
    pub half_life_days: f32,
    pub min_score: f32,
    /// 検索候補の件数上限。0以下を渡すと全件が対象になる。
    pub candidate_window: i64,
}
//バージョン情報
#[unsafe(no_mangle)]
pub extern "C" fn rugst_version() -> u32 {
    1
}
//記憶を保存
#[unsafe(no_mangle)]
pub extern "C" fn rugst_create(db_path: *const c_char) -> *mut RugstHandle {
    // nullptrの早期リターン
    if db_path.is_null() {
        return std::ptr::null_mut();
    }

    // パニックが FFI 境界を越えるのを防ぐ
    let result = catch_unwind(|| unsafe {
        // C-String から文字列スライスへの変換
        let c_str = CStr::from_ptr(db_path);
        let db_path_str = match c_str.to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        };

        // インスタンスの生成
        match Rugst::new(db_path_str) {
            Ok(rugst) => {
                // メモリをヒープに確保し、C側に所有権を放棄する
                Box::into_raw(Box::new(RugstHandle { inner: Mutex::new(rugst) }))
            }
            Err(_) => std::ptr::null_mut(),
        }
    });

    // パニックが発生した場合はヌルポインタを返す
    result.unwrap_or(std::ptr::null_mut())
}
//ポインタをrust側に戻し、メモリを解放
#[unsafe(no_mangle)]
pub extern "C" fn rugst_destroy(handle: *mut RugstHandle) {
    if handle.is_null() {
        return;
    }

    unsafe {
        drop(Box::from_raw(handle));
    }
}
//dbから会話履歴を取得
#[unsafe(no_mangle)]
pub extern "C" fn rugst_remember(
    handle: *mut RugstHandle,
    channel_id: *const c_char,
    author_id: *const c_char,
    role: *const c_char,
    content: *const c_char,
) -> RugstError {
    // ポインタの検査
    if handle.is_null()
        || channel_id.is_null()
        || author_id.is_null()
        || role.is_null()
        || content.is_null()
    {
        return RugstError::NullPointer;
    }

    // C文字列 → Rust文字列
    let channel_id = unsafe {
        match CStr::from_ptr(channel_id).to_str() {
            Ok(s) => s,
            Err(_) => return RugstError::InvalidUtf8,
        }
    };

    let author_id = unsafe {
        match CStr::from_ptr(author_id).to_str() {
            Ok(s) => s,
            Err(_) => return RugstError::InvalidUtf8,
        }
    };

    let role = unsafe {
        match CStr::from_ptr(role).to_str() {
            Ok(s) => s,
            Err(_) => return RugstError::InvalidUtf8,
        }
    };

    let content = unsafe {
        match CStr::from_ptr(content).to_str() {
            Ok(s) => s,
            Err(_) => return RugstError::InvalidUtf8,
        }
    };

    // 生ポインタ → Rustの参照
    let handle_ref = unsafe { &*handle };

    // 他スレッドがロック中にpanicしても以降ずっと死んだままにならないよう、
    // poison時は内部値を回収して継続する
    let mut rugst = handle_ref
        .inner
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    // Rust内部APIを呼ぶ
    match rugst.remember(channel_id, author_id, role, content) {
        Ok(()) => RugstError::Ok,
        Err(_) => RugstError::InternalError,
    }
}
#[repr(C)]
pub struct RugstSearchResult {
    pub text: *mut c_char,
    pub score: f32,
    pub created_at: i64,
}
#[repr(C)]
pub struct RugstSearchResults {
    pub results: *mut RugstSearchResult,
    pub len: usize,
    // 確保時の実際のcapacity。解放時は必ずこの値を使うこと。
    // len と capacity が食い違うと Vec::from_raw_parts が誤ったレイアウトで
    // 解放してしまい未定義動作になる(以前のバグ)。
    pub capacity: usize,
}
#[unsafe(no_mangle)]
pub extern "C" fn rugst_search(
    handle: *mut RugstHandle,
    channel_id: *const c_char,
    query: *const c_char,
    options: RugstSearchOptions,
) -> RugstSearchResults {
    if handle.is_null() || channel_id.is_null() || query.is_null() {
        return empty_results();
    }

    let result = catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        let channel_id = match cstr_to_str(channel_id) {
            Some(s) => s,
            None => return empty_results(),
        };

        let query = match cstr_to_str(query) {
            Some(s) => s,
            None => return empty_results(),
        };

        let handle_ref = &*handle;
        let mut rugst = handle_ref
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        let options = crate::search::SearchOptions::from(&options);

        let results = match rugst.search(channel_id, query, &options) {
            Ok(results) => results,
            Err(_) => return empty_results(),
        };

        let mut ffi_results = Vec::with_capacity(results.len());

        for result in results {
            let text = match std::ffi::CString::new(result.text) {
                Ok(text) => text.into_raw(),
                Err(_) => continue,
            };

            ffi_results.push(RugstSearchResult {
                text,
                score: result.score,
                created_at: result.created_at,
            });
        }

        let len = ffi_results.len();
        // CString::new の失敗で continue するとpushされる件数が
        // with_capacity で確保した容量より少なくなりうる。
        // forget する前に capacity を実際の長さに合わせておくことで、
        // 解放側に渡す capacity と Vec の実アロケーションを一致させる。
        ffi_results.shrink_to_fit();
        let capacity = ffi_results.capacity();
        let ptr = ffi_results.as_mut_ptr();

        std::mem::forget(ffi_results);

        RugstSearchResults {
            results: ptr,
            len,
            capacity,
        }
    }));

    result.unwrap_or_else(|_| empty_results())
}
//ヘルパー関数2つ
// 1. C-chrをstrに変換
unsafe fn cstr_to_str<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }

    unsafe {
        CStr::from_ptr(ptr).to_str().ok()
    }
}
// 2. 空の検索結果を作る
fn empty_results() -> RugstSearchResults {
    RugstSearchResults {
        results: std::ptr::null_mut(),
        len: 0,
        capacity: 0,
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn rugst_free_search_results(
    results: RugstSearchResults,
) {
    if results.results.is_null() {
        return;
    }

    unsafe {
        // 確保時と同じ capacity を使って解放する(len ではなく capacity)
        let results_vec = Vec::from_raw_parts(
            results.results,
            results.len,
            results.capacity,
        );

        for result in &results_vec {
            if !result.text.is_null() {
                drop(std::ffi::CString::from_raw(result.text));
            }
        }

        drop(results_vec);
    }
}