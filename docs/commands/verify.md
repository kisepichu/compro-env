# ce verify

## 概要

`ce verify` は公開 solution を対応する OJ に投げて verification record を最新の状態に保つコマンド。
LibraryChecker のような unattended trackable OJ で自動判定を回し、fingerprint の変化を検知して再検証する。詳細な設計は `docs/spec.md` §7.2 / §8 / §10 を参照。

- 結果は端末または CI ログにのみ出力する。`verification/results/**/*.json` の書き換えは `ce verify` の副作用として発生する。
- `ce verify` は「その時点で走っている attempt」を先に片付けてから新規計画を組み立てる (resume-first)。
- One-in-flight-per-OJ: 同じ OJ に対して同一 invocation 中で複数の新規投稿は行わない。
- 現在の `ce verify` は Unix-like shell (`sh`) が利用できる環境のみ対応する (`ce check` と同じ制約)。

## シグネチャ

```
ce verify [solution-id]
```

- `[solution-id]`: `librarychecker-aplusb/aplusb/main` のような 3 セグメントの solution id。省略時は discovery manifest の `publish = true` かつ `[verify]` を持つすべての solution を対象にする。
- 未設定 (`[verify]` が無い) solution が明示的に指定された場合は `not configured; skipping` を出力してその solution だけスキップする (他 solution の失敗が無ければ exit 0)。
- 未知の solution が指定された場合はエラー終了する。

## 挙動

1. `templates/` を含む project root を探索し、`config.toml` の `[library]` を strict にロードする。
2. Discovery + normalized analysis を走らせて `AnalysisSnapshot` を作る。
3. **Resume phase.** すべての published solution に対して `submission_lifecycle::resume_pending` を呼び、`Starting` / `AcceptanceUnknown` / `Submitted` / `Queued` / `Judging` / `InfrastructureFailure` の record を可能な限り前進させる。
4. Resume で touch された OJ (pre-resume の時点で non-terminal な record を持っていた OJ) は、この tick では新規開始しない (spec §8.3)。
5. 対象 solution ごとに `[verify].libraries` 直接依存と transitive closure の source bytes を集めて fingerprint を計算する。
6. **Stale/never gate.** 既存 record の fingerprint と比較する:
   - `Completed(Accepted)` かつ一致 → skip (verified)。
   - `Completed(WrongAnswer 等)` / `Unavailable` かつ一致 → 再実行はせず、exit code だけ 1 にする (spec §10)。
   - fingerprint が変わっていれば stale とみなして新規計画を組む。
   - Resume が今 tick で terminal に到達させたものは fingerprint 比較をスキップしてその terminal を最終状態とする。
7. **Check + test barrier.** 新規開始候補になった solution 群の distinct な `LanguageId` を集めて各言語 1 回だけ `run_checks` を実行する。続けて各 solution の `test_command` を `sh -c` で走らせる。どれか 1 つでも失敗すればこの tick の新規開始は全部止める (resume は影響を受けない)。
8. Barrier をくぐり抜けた solution は preprocess hook (`Config::submit_preprocess()`) を通し、`SubmissionPlan` を構築、`start_plan` で `Starting` record を持続してから starter を呼ぶ。`Trackable` なら即座に `poll_handle` で 15 分 budget の範囲で追跡する。
9. 各 solution について stdout に 1 行の status を出す。

## 環境変数

Preprocess hook は `ce submit` と同じ形式で呼び出す:

| 変数                | 内容                                                                     |
| ------------------- | ------------------------------------------------------------------------ |
| `CE_LANGUAGE`       | 対象 solution の `LanguageId`                                            |
| `CE_OJ`             | 対象 OJ (`atcoder` / `librarychecker`)                                   |
| `CE_CONTEST_ID`     | `SolutionId` の contest 部分                                             |
| `CE_PROBLEM_CODE`   | `SolutionId` の problem 部分                                             |
| `CE_SOLUTION_ID`    | 完全な `SolutionId` (spec §11 の SolutionId key)                         |
| `CE_SOLUTION_ENTRY` | project root からの entry file 相対パス                                  |
| `PATH` / `HOME` / `TERM` | 親プロセスから継承                                                  |

各 solution の `test_command` は `sh -c` を通じて `<repository_root>/<published.root>` を CWD として起動する。環境変数は `PATH` / `HOME` / `TERM` に加えて `CE_REPOSITORY_ROOT` と `CE_SOLUTION_ID` を渡す。

## Exit code (spec §10)

- 全 status が `verified` または `not configured` → **exit 0**
- 1 件でも `rejected` / `unavailable` / `pending` (Queued/Judging/AcceptanceUnknown/Starting/InfrastructureFailure) / `infrastructure error` / `budget exhausted` / `test failed` / `language check failed` があれば → **exit 1**

## AtCoder

AtCoder は spec §9 で `InteractiveUntrackable` に分類されている。`ce verify` は unattended trackable なアダプタしか受け付けないため、AtCoder 向けの status は必ず `Unavailable (InteractiveUntrackable)` となり、run 全体としては exit 1 になる。ブラウザ経由の submission が必要なケースは `ce submit` を使う。

## 例

```
$ ce verify
[abc999/a/main] unavailable (interactive-only OJ requires user action)
[librarychecker-aplusb/aplusb/main] verified
```

```
$ ce verify librarychecker-aplusb/aplusb/main
[librarychecker-aplusb/aplusb/main] verified
```

```
$ ce verify abc999/a/main
[abc999/a/main] unavailable (interactive-only OJ requires user action)
$ echo $?
1
```

## 内部コマンド

`ce internal verify-prepare` / `ce internal verify-start` / `ce internal verify-poll` は CI が prepare / start / poll を独立 job で実行するための hidden entrypoint。`ce --help` には出さない。使い方は CI パイプラインの仕様書側にまとめる。

## 関連

- `docs/commands/check.md` — 各言語の `check_command` を verify の barrier で共有する。
- `docs/commands/submit.md` — AtCoder 手動投稿。
- `docs/spec.md` §7.2 / §8 / §10 — verify の設計要件。
