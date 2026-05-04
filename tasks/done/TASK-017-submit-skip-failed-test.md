# TASK-017: submit skips failed tests

## 参照仕様

- docs/commands/submit.md
- docs/commands/test.md

## チェックリスト

- [x] 既存仕様と実装の submit/test 連携を確認する
- [x] `ce submit` が提出 URL 生成前に `ce test` 相当を実行する仕様へ更新する
- [x] test 仕様の「将来」記述を現行仕様に同期する
- [x] `Service::submit` でテスト失敗時に提出 URL を生成しない
- [x] submit サービスのユニットテストを追加・更新する
- [x] `cargo test --workspace` で検証する

## 完了条件

- [x] `test_command` が exit 0 の場合だけ submit URL が返る
- [x] `test_command` が 0 以外の場合はエラーになり、ブラウザ起動側に URL が渡らない
- [x] 仕様に submit 前テストの扱いが明記されている

## 作業ログ

- 2026-05-03: 作業開始
- 2026-05-03: 仕様更新、実装、ユニットテスト追加、実ケース確認、検証完了
- 2026-05-03: PR review 対応として非 Unix では提出前テストをスキップし、既存の submit URL 生成を維持するよう修正
