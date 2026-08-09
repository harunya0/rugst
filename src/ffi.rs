use crate::Rugst;
use std::ffi::{c_char, CStr};
use std::panic::catch_unwind;

impl From<&RugstSearchOptions> for crate::search::SearchOptions {
    fn from(value: &RugstSearchOptions) -> Self {
        Self {
            top_k: value.top_k as usize,
            half_life_days: value.half_life_days,
            min_score: value.min_score,
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
#[repr(C)]
pub struct RugstHandle {
    inner: Rugst,
}
//検索オプション
#[repr(C)]
pub struct RugstSearchOptions {
    pub top_k: u32,
    pub half_life_days: f32,
    pub min_score: f32,
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
                Box::into_raw(Box::new(RugstHandle { inner: rugst }))
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
    let rugst = unsafe {
        &mut (*handle).inner
    };

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
}
#[unsafe(no_mangle)]
pub extern "C" fn rugst_search(
    handle: *mut RugstHandle,
    channel_id: *const c_char,
    query: *const c_char,
    options: RugstSearchOptions,
) -> RugstSearchResults {
    if handle.is_null() || channel_id.is_null() || query.is_null() {
        return RugstSearchResults {
            results: std::ptr::null_mut(),
            len: 0,
        };
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

        let rugst = &mut (*handle).inner;

        let options = crate::search::SearchOptions::from(&options);

        let results = match rugst.search(channel_id, query, &options) {
            Ok(results) => results,
            Err(_) => {
                return RugstSearchResults {
                    results: std::ptr::null_mut(),
                    len: 0,
                };
            }
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

        let ptr = ffi_results.as_mut_ptr();

        std::mem::forget(ffi_results);

        RugstSearchResults {
            results: ptr,
            len,
        }
    }));

    result.unwrap_or(RugstSearchResults {
        results: std::ptr::null_mut(),
        len: 0,
    })
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
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn rugst_free_search_results(
    results: RugstSearchResults,
) {
    if results.results.is_null() || results.len == 0 {
        return;
    }

    unsafe {
        let results_vec = Vec::from_raw_parts(
            results.results,
            results.len,
            results.len,
        );

        for result in &results_vec {
            if !result.text.is_null() {
                drop(std::ffi::CString::from_raw(result.text));
            }
        }

        drop(results_vec);
    }
}