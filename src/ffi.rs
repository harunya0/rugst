use crate::Rugst;
use std::ffi::{c_char, CStr};
use std::panic::catch_unwind;

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

// Rugst 自体が内部で(埋め込み用・DB用それぞれ別々に)ロックを持つように
// なったため、Rugst は Sync になる。以前のように RugstHandle 側で
// Mutex<Rugst> を被せて「呼び出し全体」を1本のロックで直列化する
// 必要はなくなった。これにより、埋め込み計算とDBアクセスが互いを
// ブロックしなくなり、複数スレッドからの呼び出しがより並行に進む。
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
    let result = catch_unwind(|| {
        // SAFETY: db_path は直前に null チェック済み。C呼び出し側は
        // rugst_create に渡す間、db_path が有効なNUL終端C文字列を指す
        // ポインタであり続けることを保証する契約になっている
        // (この関数のシグネチャ上の前提)。
        unsafe {
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

    // SAFETY: handle は null チェック済み。呼び出し契約として、
    // handle は rugst_create が返した有効なポインタであり、
    // rugst_destroy は各ハンドルにつき一度しか呼ばれないことを
    // 呼び出し側(C)が保証する必要がある(二重解放はUB)。
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

    // SAFETY: channel_id は null チェック済み。呼び出し側は有効な
    // NUL終端C文字列を指すポインタを渡す契約になっている。
    let channel_id = unsafe {
        match CStr::from_ptr(channel_id).to_str() {
            Ok(s) => s,
            Err(_) => return RugstError::InvalidUtf8,
        }
    };

    // SAFETY: author_id も同様にnullチェック済みで、呼び出し側が
    // 有効なNUL終端C文字列であることを保証する契約。
    let author_id = unsafe {
        match CStr::from_ptr(author_id).to_str() {
            Ok(s) => s,
            Err(_) => return RugstError::InvalidUtf8,
        }
    };

    // SAFETY: role も同様。
    let role = unsafe {
        match CStr::from_ptr(role).to_str() {
            Ok(s) => s,
            Err(_) => return RugstError::InvalidUtf8,
        }
    };

    // SAFETY: content も同様。
    let content = unsafe {
        match CStr::from_ptr(content).to_str() {
            Ok(s) => s,
            Err(_) => return RugstError::InvalidUtf8,
        }
    };

    // SAFETY: handle は null チェック済みで、rugst_create が返した
    // 有効なポインタであることを呼び出し側が保証する契約。
    // Rugst は内部の埋め込み用・DB用ロックによりSyncなので、
    // 共有参照から直接メソッドを呼び出して問題ない。
    let handle_ref = unsafe { &*handle };

    // Rust内部APIを呼ぶ(remember は &self で足りる)
    match handle_ref.inner.remember(channel_id, author_id, role, content) {
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
    // 解放してしまい未定義動作になる。
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

    // SAFETY: handle/channel_id/query は直前にnullチェック済み。
    // 呼び出し側は有効なポインタ(handleはrugst_createが返したもの、
    // channel_id/queryは有効なNUL終端C文字列)を渡す契約になっている。
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

        let options = crate::search::SearchOptions::from(&options);

        // search も &self で足りる(embedding用ロックとDB用ロックは
        // それぞれの内部で個別に取得・解放される)
        let results = match handle_ref.inner.search(channel_id, query, &options) {
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
//
// SAFETY: 呼び出し側は、ptrがnullでない場合は有効なNUL終端C文字列を
// 指すポインタであることを保証する必要がある。この関数自体は
// nullチェックのみ行い、ポインタの有効性そのものは呼び出し契約に依存する。
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

    // SAFETY: results は rugst_search が返した RugstSearchResults を
    // そのまま渡す契約(len/capacity/resultsのいずれも書き換えないこと)。
    // capacity は確保時にshrink_to_fit済みの実際の値なので、
    // Vec::from_raw_parts に渡すレイアウトは確保時と一致する。
    // また各要素のtextはCString::into_rawで生成されたポインタなので
    // CString::from_rawで解放できる。この関数は各ハンドルにつき
    // 一度しか呼ばれない契約(二重解放はUB)。
    unsafe {
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

#[repr(C)]
pub struct RugstListItem {
    pub id: i64,
    pub text: *mut c_char,
    pub created_at: i64,
}

#[repr(C)]
pub struct RugstListResults {
    pub items: *mut RugstListItem,
    pub len: usize,
    pub capacity: usize,
}

fn empty_list_results() -> RugstListResults {
    RugstListResults { items: std::ptr::null_mut(), len: 0, capacity: 0 }
}

/// role を指定してレコードを一覧取得する(事実管理画面用)。
#[unsafe(no_mangle)]
pub extern "C" fn rugst_list(
    handle: *mut RugstHandle,
    channel_id: *const c_char,
    role: *const c_char,
) -> RugstListResults {
    if handle.is_null() || channel_id.is_null() || role.is_null() {
        return empty_list_results();
    }

    let result = catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        let channel_id = match cstr_to_str(channel_id) {
            Some(s) => s,
            None => return empty_list_results(),
        };
        let role = match cstr_to_str(role) {
            Some(s) => s,
            None => return empty_list_results(),
        };

        let handle_ref = &*handle;
        let rows = match handle_ref.inner.list_by_role(channel_id, role) {
            Ok(rows) => rows,
            Err(_) => return empty_list_results(),
        };

        let mut items = Vec::with_capacity(rows.len());
        for (id, content, created_at) in rows {
            let text = match std::ffi::CString::new(content) {
                Ok(t) => t.into_raw(),
                Err(_) => continue,
            };
            items.push(RugstListItem { id, text, created_at });
        }

        let len = items.len();
        items.shrink_to_fit();
        let capacity = items.capacity();
        let ptr = items.as_mut_ptr();
        std::mem::forget(items);

        RugstListResults { items: ptr, len, capacity }
    }));

    result.unwrap_or_else(|_| empty_list_results())
}

#[unsafe(no_mangle)]
pub extern "C" fn rugst_free_list_results(results: RugstListResults) {
    if results.items.is_null() {
        return;
    }
    unsafe {
        let items_vec = Vec::from_raw_parts(results.items, results.len, results.capacity);
        for item in &items_vec {
            if !item.text.is_null() {
                drop(std::ffi::CString::from_raw(item.text));
            }
        }
        drop(items_vec);
    }
}

/// idを指定してレコードを削除する。
#[unsafe(no_mangle)]
pub extern "C" fn rugst_delete(handle: *mut RugstHandle, id: i64) -> RugstError {
    if handle.is_null() {
        return RugstError::NullPointer;
    }
    let handle_ref = unsafe { &*handle };
    match handle_ref.inner.delete(id) {
        Ok(_) => RugstError::Ok,
        Err(_) => RugstError::InternalError,
    }
}

/// idを指定して本文を更新する(embeddingも再計算)。
#[unsafe(no_mangle)]
pub extern "C" fn rugst_update(
    handle: *mut RugstHandle,
    id: i64,
    content: *const c_char,
) -> RugstError {
    if handle.is_null() || content.is_null() {
        return RugstError::NullPointer;
    }

    let content = unsafe {
        match CStr::from_ptr(content).to_str() {
            Ok(s) => s,
            Err(_) => return RugstError::InvalidUtf8,
        }
    };

    let handle_ref = unsafe { &*handle };
    match handle_ref.inner.update(id, content) {
        Ok(_) => RugstError::Ok,
        Err(_) => RugstError::InternalError,
    }
}