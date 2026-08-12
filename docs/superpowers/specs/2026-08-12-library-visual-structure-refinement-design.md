# Library visual structure refinement design

最終更新: 2026-08-12

- 状態: 設計承認済み
- 対象 gate: Human gate G1 visual design integration
- 関連資料:
  - [Library platform design](2026-08-10-library-platform-design.md)
  - [Library Web semantic structure handoff](2026-08-10-library-web-structure-handoff.md)

## 1. 目的

初回 visual design で見つかった、CSS だけでは解消できない情報順、日時表記、見出し階層の不整合を直す。
route、公開データ、検索仕様、source viewer contract、必須 JavaScript は変更しない。

## 2. 採用方針

### 2.1 Home の recent list

Recently solved solutions の item は次の DOM 順にする。

```text
solution title
language
contest ID / problem code
solved date
verification status
```

Recently updated libraries の `title / language / path / date / status` と列の意味を揃える。
CSS の `order` ではなく renderer の情報順を変更し、DOM、読み上げ、視覚の順序を一致させる。

### 2.2 日時表示

すべての `<time>` は元の RFC 3339 値を `datetime` 属性に保持する。表示 text だけを UTC へ正規化する。

- 一覧、recent list、browse card、集約値: `YYYY-MM-DD`
- library / solution detail、verification evidence、verification result: `YYYY-MM-DD HH:mm UTC`

無効な timestamp が renderer に渡った場合は例外にせず、元の値を表示 text に使う。strict な公開データ検証は従来どおり上流の責務とする。

日時整形は共通 helper に集約し、元データや sort key は変更しない。

### 2.3 Library verification evidence

各 evidence row は次の DOM 順にする。

```text
solution identity link
judged time / OJ submission link
verification status
optional stale reason
```

solution identity は `solution_page_id` に一致する公開 solution detail への base-aware link とし、
表示 text は完全な `solution_id` を保つ。evidence が公開 solution に解決できなければ、壊れた link や
plain text へ縮退せず site build を失敗させる。

desktop では status badge を最終 column の右端へ置く。mobile でも同じ DOM 順を保ち、stale reason は row 全幅に表示する。

### 2.4 Libraries / Solutions root

`/libraries/` の `Languages` と `/solutions/` の `Contests` は、唯一の内容群を言い換える冗長な見出しなので削除する。

```text
main
|- page header > h1 Libraries | Solutions
`- ul > li > article
```

対象が 0 件なら `ul` の代わりに既存の empty-state message を page header 直下へ置く。見出しのない `section` は残さず、list と empty state を意味のない layout wrapper で囲わない。

### 2.5 Browse page header の重複 metadata

breadcrumb が同じ階層情報を示すため、次の page header subtitle は削除する。

- contest page の `Contest`
- problem page の `contest ID / problem code`
- library language / category page の language ID または root-relative category path

page の `h1`、breadcrumb、document title、canonical URL は維持する。削除した path を別の場所へ重複表示しない。

### 2.6 Problem page の solution row

problem page の solution list は card grid ではなく全幅の row list とする。各 row の DOM 順は次のとおり。

```text
solution name link
language
solved date
direct public dependency count
verification status
```

desktop では status badge を最終 column の右端へ置く。mobile では DOM 順を変えずに折り返し、
solution name を row の先頭に保つ。section heading `Solutions` と既存の sort は維持する。

### 2.7 Solution detail header

breadcrumb と重複する contest / problem path は page header から削除する。header は `h1` に続けて、
次の 1 metadata row を持つ。

```text
language
online judge
solved time
verification status
```

desktop では 1 行に配置し、狭い画面では同じ DOM 順のまま折り返す。

### 2.8 Solution detail の library 表示

`verifies` と `direct_dependencies` は内部データでは引き続き別の概念として保持する。
ただし solution を閲覧する利用者は verify target を確認しないものとし、solution detail では
`verifies` を表示しない。library detail の verification evidence を verify relationship の公開 UI とする。

solution detail の in-page navigation と section heading は `Depends on` とし、direct public dependency list だけを
表示する。内側の `Verifies` / `Depends on` subheading は置かない。既存 fragment を壊さないため
`section#libraries` は維持し、private dependency note も維持する。

## 3. `tmp/additional.md` の判断

| 提案 | 判断 | 理由 |
| --- | --- | --- |
| P1 route entry から CSS を import | 現状維持 | 共有 document renderer の base-aware stylesheet link で全 route と root / project base を一元検証できる。route entry の重複 head 管理を増やさない。 |
| P2 dark mode | 別 task | 今回承認された light pastel design と別 theme で、Shiki の dual theme も必要になる。 |
| P3 source の行全幅 highlight | 別 task | newline と copy/paste の source contract を変えるため、source viewer 単独で設計・検証する。 |
| P4 時刻の短縮表示 | 採用 | `datetime` を保持して表示 text のみ短縮する。 |
| P5 filter chip の除去操作 | 別 task | query 書き換えを伴う検索機能追加であり、G1 の markup / CSS integration 外。 |
| P6 copy / wrap / line range | 別 task | handoff で MVP 対象外と明記された source viewer 機能。 |
| P7 数式の実描画 | 別 task | Markdown pipeline、sanitize allowlist、self-host asset の設計が必要。 |
| P8 status tooltip | 採用しない | hover 依存を避け、既存の text label と detail callout を維持する。 |
| P9 card の Latest solved を日付だけにする | 採用 | browse card の compact date 規則へ統合する。 |
| P10 Home の language 位置を揃える | 採用 | renderer の DOM 順を変更し、両 recent list を同じ列構造にする。 |

## 4. 構造資料の更新

実装と同じ変更で以下の normative documentation を更新する。

- `2026-08-10-library-web-structure-handoff.md`
  - Home item の情報順
  - root page の見出しなし list 構造
  - compact / detailed time 表示 contract
  - verification evidence の solution link と status 最終順
  - browse header の重複 subtitle 削除と problem solution row
  - solution detail header と Depends on section
- `2026-08-10-library-platform-design.md`
  - Home recent item と solution browse の表示項目
  - library verification evidence の順序
  - timestamp の値と表示 text の責務分離
  - browse / detail の重複 metadata 削除
  - `verifies` を保持しつつ solution detail では表示しない UI 方針

## 5. 検証

renderer test で次を固定する。

- Home recent solution の DOM 順が `title / language / contest-problem / time / status`。
- compact time の text が日付だけで、`datetime` が元値のまま。
- detail / evidence time の text が UTC 分単位で、`datetime` が元値のまま。
- verification evidence の solution link が対応する base-aware detail URL を指し、status badge が row の最後にある。
- evidence が公開 solution に解決できない場合は render / build が失敗する。
- Libraries / Solutions root に冗長な `h2` と `section` がなく、list または empty state が page header に続く。
- contest、problem、library category page header に重複 subtitle がない。
- problem solution row が `name / language / time / dependency count / status` の DOM 順を持つ。
- solution detail header が `language / OJ / time / status` の 1 metadata row だけを持つ。
- solution detail が `verifies` を表示せず、`Depends on` の direct dependency list だけを表示する。
- root `/` と project base `/compro-env/` の build / semantic checks が通る。

CSS は Home の subgrid と verification evidence の column 順を新しい DOM contract に合わせる。route、breadcrumb、Pagefind 属性、source line ID、status label は変更しない。
