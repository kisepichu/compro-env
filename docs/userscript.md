# Tampermonkey Userscript

`ce submit` が開く AtCoder 提出ページで、問題選択・ソースコード注入・言語選択を自動化する userscript の仕様。

## 動作概要

1. `ce submit` が提出ページを URL フラグメント付きで開く:
   ```
   https://atcoder.jp/contests/abc334/submit?taskScreenName=abc334_a#ce=BASE64
   ```
2. Tampermonkey userscript が `#ce=` フラグメントを検出し、以下を自動で行う:
   - 問題プルダウン (`data.TaskScreenName`) の選択 (`?taskScreenName=` クエリパラメータでも選択されているが、select2 の UI 同期のため JS イベントを発火する)
   - 言語プルダウンの選択 (select2 ウィジェット経由)
   - ソースコードの textarea への注入
3. ユーザーが内容を確認して Submit ボタンを押す

自動提出はしない。Turnstile チャレンジはユーザーがブラウザで通過する。

## URL フラグメント仕様

```
#ce=<URL-safe base64 (RFC 4648 §5、パディングあり)>
```

デコード後の JSON:

```json
{
  "lang_id": "6088",
  "source": "<ソースコード全文>"
}
```

- `lang_id`: AtCoder の言語 ID (文字列)
- `source`: 提出するソースコード全文

問題 ID は `?taskScreenName=` クエリパラメータから取得する。

## Tampermonkey スクリプト仕様

### メタデータ

```js
// @name         ce submit helper
// @namespace    https://github.com/kisepichu/compro-env
// @version      1.0
// @description  Auto-fill AtCoder submit form from ce submit URL fragment
// @author       kisepichu
// @match        https://atcoder.jp/contests/*/submit*
// @grant        none
// @run-at       document-idle
```

### 処理フロー

1. `location.hash` が `#ce=` で始まるか確認する。始まらない場合は何もしない
2. `#ce=` 以降を URL-safe base64 デコードして JSON をパースする
3. `location.search` から `taskScreenName` パラメータを取得する
4. ページの `select#select-task` select2 ウィジェットで `taskScreenName` を選択し、`change` イベントを発火する
   - これにより `div#select-lang-{taskScreenName}` が表示される
5. `div#select-lang-{taskScreenName}` 内の `<select>` で `lang_id` を選択し、`change` イベントを発火する
6. `textarea[name=sourceCode]` に `source` を注入する
7. URL フラグメントを消去する (`history.replaceState` で `#` なし URL に変更)

### select2 の操作

AtCoder の言語プルダウンは select2 ライブラリを使っている。直接 `value` をセットするだけでは UI が更新されないため、jQuery の `.trigger('change')` が必要:

```js
$(selectEl).val(langId).trigger('change');
```

### 注意事項

- select2 の初期化完了を待つため、`document-idle` 後も `MutationObserver` または `setTimeout` でリトライが必要な場合がある
- `div#select-lang-{taskScreenName}` は問題選択後に `display:none` → `display:block` になる。`change` イベント発火後に表示されることを確認してから言語を選択する

## インストール方法

1. ブラウザに [Tampermonkey](https://www.tampermonkey.net/) を導入する
2. Tampermonkey ダッシュボード → 新しいスクリプトを追加
3. 上記仕様に基づいたスクリプトを貼り付けて保存する

実際のスクリプトコードは `scripts/atcoder-submit-helper.user.js` にある。
