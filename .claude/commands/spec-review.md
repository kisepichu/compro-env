実装と仕様を照合し、乖離があれば修正する。

## 手順

1. 引数からコマンド名を取得する (例: `/spec-review submit` → `submit`)
   - 引数がなければ「どのコマンドをレビューしますか?」と聞く
2. `docs/commands/{command}.md` を読む
3. 関連する実装ファイルを読む
   - `crates/domain/src/` のエンティティ
   - `crates/usecases/src/service/{command}.rs` など
   - `crates/infrastructure/src/` の実装
4. 仕様と実装の差異を列挙して報告する:
   - 仕様にあるが未実装のもの
   - 実装にあるが仕様にないもの
   - 挙動が仕様と異なるもの
5. ユーザーに確認し、どちらを正とするかを決めて修正する:
   - 仕様が正しければ実装を修正する
   - 実装が正しければ `/spec-update {command}` の手順で仕様を更新する
6. 対応するタスクファイル (`tasks/doing/TASK-*-{command}.md`) があればチェックリストを更新する

## 注意

- 小さな差異 (変数名・コメントなど) は無視してよい
- アーキテクチャルール違反 (`CLAUDE.md` 参照) があれば必ず報告する
