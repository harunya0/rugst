#ifndef RUGST_H
#define RUGST_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct RugstHandle RugstHandle;

typedef enum {
    RUGST_OK = 0,
    RUGST_NULL_POINTER = 1,
    RUGST_INVALID_UTF8 = 2,
    RUGST_INTERNAL_ERROR = 3
} RugstError;

typedef struct {
    uint32_t top_k;
    float half_life_days;
    float min_score;
    /* 検索候補の件数上限。0以下を渡すと全件(履歴すべて)が対象になる。 */
    int64_t candidate_window;
    /* ハイブリッド検索(ベクトル類似度 + FTS5のBM25キーワード検索をRRFで
     * 統合)を有効にするか。0=無効(従来通りベクトルのみ)、それ以外=有効。 */
    int32_t enable_fts;
    /* RRF(Reciprocal Rank Fusion)のkパラメータ。0以下を渡すと既定値(60)が
     * 使われる。値が大きいほど下位の順位の影響が均される。 */
    uint32_t rrf_k;
    /* FTS5側のRRFスコアに掛ける重み。0以下を渡すと既定値(1.0)が使われる。
     * 大きいほどキーワード一致を、小さいほど意味的類似度を重視する。 */
    float fts_weight;
} RugstSearchOptions;

typedef struct {
    int64_t id;
    char* text;
    float score;
    int64_t created_at;
} RugstSearchResult;

typedef struct {
    RugstSearchResult* results;
    size_t len;
    /* rugst_search 側で確保された実際のcapacity。
     * rugst_free_search_results はこの値を使って解放するため、
     * 呼び出し側で書き換えないこと。 */
    size_t capacity;
} RugstSearchResults;

uint32_t rugst_version(void);

RugstHandle* rugst_create(const char* db_path);

void rugst_destroy(RugstHandle* handle);

RugstError rugst_remember(
    RugstHandle* handle,
    const char* channel_id,
    const char* author_id,
    const char* role,
    const char* content
);

RugstSearchResults rugst_search(
    RugstHandle* handle,
    const char* channel_id,
    /* 検索対象を絞るrole ("fact" など)。user/assistantの過去発言を
     * 検索候補から除外するために必須。 */
    const char* role,
    const char* query,
    RugstSearchOptions options
);

void rugst_free_search_results(
    RugstSearchResults results
);

typedef struct {
    int64_t id;
    char* text;
    int64_t created_at;
} RugstListItem;

typedef struct {
    RugstListItem* items;
    size_t len;
    size_t capacity;
} RugstListResults;

RugstListResults rugst_list(
    RugstHandle* handle,
    const char* channel_id,
    const char* role
);

void rugst_free_list_results(RugstListResults results);

RugstError rugst_delete(RugstHandle* handle, int64_t id);

RugstError rugst_update(
    RugstHandle* handle,
    int64_t id,
    const char* content
);

typedef struct {
    char* role;
    char* content;
} RugstHistoryItem;

typedef struct {
    RugstHistoryItem* items;
    size_t len;
    size_t capacity;
} RugstHistoryResults;

/* 指定チャンネルの直近の会話履歴を古い順(時系列順)で取得する
 * (AIへのプロンプト用)。limitが会話全体の件数上限。 */
RugstHistoryResults rugst_get_recent_history(
    RugstHandle* handle,
    const char* channel_id,
    int64_t limit
);

void rugst_free_history_results(RugstHistoryResults results);

#ifdef __cplusplus
}
#endif

#endif