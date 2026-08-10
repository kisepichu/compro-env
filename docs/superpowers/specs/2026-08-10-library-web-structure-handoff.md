# Library Web semantic structure handoff

最終更新: 2026-08-10

- 状態: 設計確定・visual design integration 用 companion
- 主設計:
  [Library platform design](2026-08-10-library-platform-design.md)

## 1. この資料の目的

compro-env の library / solution 静的サイトに視覚デザインを適用するための構造資料である。
ここでは色、font、余白、border、shadow、column 幅などの視覚表現を指定しない。

デザインで変更してよいもの:

- 色、typography、spacing、border、background、icon の形
- desktop / mobile の grid、column、card 配置
- 意味を持たない wrapper と class の追加
- sticky 表示など、元の内容を DOM から除去しない視覚的拡張

変更しないもの:

- route と query parameter
- landmark、見出し階層、list / table の意味
- component 内の情報順
- source line ID、permalink、検索属性
- status の意味と text label
- 公開・非公開データの境界
- JavaScript がなくても成立する通常 navigation

サイトは Astro で全通常 page を静的生成し、検索結果だけ Pagefind の JavaScript API で
browser 上に描画する。

## 2. Route

```text
/                                      Home
/search/                               Search
/libraries/                            Libraries
/libraries/{lang}/                     Language
/libraries/{lang}/{directory...}/      Library directory
/libraries/{lang}/{source-path...}/    Library detail
/solutions/                            Solutions
/solutions/{contest}/                  Contest
/solutions/{contest}/{problem}/        Problem
/solutions/{contest}/{problem}/{name}/ Solution detail
/404.html                              Not found
```

- 通常 page の canonical URL は末尾 `/` あり。
- path segment は個別に percent-encode する。
- root `/` と project base `/compro-env/` の両方で動作する。
- link、asset、search action は base path helper から生成する。
- 404 だけは artifact root の `404.html` とする。
- library / solution の rename 前 route は生成せず、redirect せず 404 にする。
- public から private または削除した detail の旧 route も redirect しない。

全 page の head contract:

- Home title は site title、他 page は `{page title} | {site title}`。
- description、canonical URL、basic Open Graph metadata を持つ。
- `<html lang>` は project-local BCP 47 language 設定を使う。
- Search と 404 は noindex。
- visual OG image は MVP 対象外。

## 3. 全 page 共通 shell

```text
body
|- a.skip-link[href="#main-content"]
|- header.site-header
|  |- a.site-title[href="{home-url}"]
|  |- nav.primary-navigation[aria-label="Primary"]
|  |  `- ul
|  |     |- li > a Libraries
|  |     |- li > a Solutions
|  |     `- li > a Search
|  `- form.global-search[role="search"][method="get"][action="{search-url}"]
|     |- label[for="global-search-query"]
|     |- input#global-search-query[name="q"][type="search"]
|     `- button[type="submit"]
|- main#main-content
|  |- nav.breadcrumb[aria-label="Breadcrumb"]
|  |- header.page-header
|  |  |- h1
|  |  |- optional summary
|  |  `- optional status / metadata
|  `- page-specific content
`- footer.site-footer
   |- repository link
   `- build source commit SHA
```

共通規則:

- `{home-url}` は base-aware な Home URL、`{search-url}` は base-aware な `/search/` URL とする。
- page ごとの `h1` は 1 件だけ。
- primary navigation と breadcrumb は `ul` / `ol` を使う。
- 現在位置に `aria-current` を付ける。
- breadcrumb の現在 page は link にしない。
- global search は `GET /search/?q=...`。
- mobile でも同じ semantic DOM を使い、CSS で折り返す。
- 必須 hamburger menu は設けない。
- skip link と全操作要素に視認可能な focus style が必要。
- navigation、breadcrumb、footer は `data-pagefind-ignore`。

## 4. Home

```text
main
|- page header
|  |- h1
|  `- repository summary
|- section.status-overview
|  |- h2
|  `- dl
|- section.languages
|  |- h2
|  `- ul > li > article.language-card
|- section.recent-libraries
|  |- h2
|  `- ul > li > article.library-card
|- section.recent-solutions
|  |- h2
|  `- ul > li > article.solution-card
`- section.attention-required
   |- h2
   `- ul > li > article.attention-card
```

- status overview は公開 library 数、公開 solution 数、verify 状態別件数を持つ。
- language card は表示名、language ID、公開 library 数、verify 状態別件数を持つ。
- recent library / solution は最大 10 件。
- attention は stale、rejected、unavailable、公開対象の解析失敗を最大 10 件。
- 0 件の section も heading と短い empty-state message を残す。
- Home 専用 search form は置かない。
- page 全体を Pagefind index から除外する。

## 5. Libraries root

```text
main
|- page header > h1 Libraries
`- section.languages
   |- h2
   `- ul > li > article.language-card
```

- Home と同じ language card を使う。
- 公開 library がある、または root description がある言語だけを表示する。
- 対象 0 件では empty-state message を表示する。
- Pagefind index から除外する。

## 6. Language / library directory

```text
main
|- page header
|  |- h1
|  |- language ID または root-relative path
|  `- public verify status overview
|- optional Markdown body
|- section.child-directories
|  |- h2
|  `- ul > li > article.directory-card
`- section.library-files
   |- h2
   `- ul > li > article.library-card
```

Directory card:

- title
- root-relative path
- public descendant count
- public verify status counts

Library card:

- title と detail link
- file name
- updated time
- verify status badge

共通規則:

- child directory と library file は混在させない。
- 直下の項目だけを表示する。
- 空 directory は library section に empty-state message を表示する。
- private library は item、件数、状態のどこにも含めない。
- Pagefind index から除外する。

## 7. Library detail

```text
article.library-detail
|- page header
|  |- h1
|  |- language / relative path / updated time
|  |- verification status badge
|  |- dependency analysis status badge
|  `- symbol analysis status badge
|- nav.in-page-navigation
|- optional div#documentation
|- section#symbols
|- section#source
|- section#dependencies
|  |- direct dependencies
|  `- direct dependents
|- section#relations
|- section#verification
`- section#diagnostics
```

- in-page navigation は存在する block / section だけへ link する。
- section ID は固定。
- documentation が空なら wrapper と navigation item を省略する。
- tab に分割せず、全内容を同じ article に残す。
- article が Pagefind body。
- header metadata は Pagefind filter / metadata として登録する。

状態別表示:

- symbol complete + 0 件: 正常な empty-state message。
- symbol partial / failed: warning と diagnostics への link。
- private dependency: 名前、path、件数を出さず、存在することだけを示す。
- external dependency: MVP では非表示。
- stale: verification section に公開可能な理由を表示。
- location 付き diagnostic: source line anchor へ link。
- private dependency を指す location: target 情報を出さず共通 reason だけを表示。
- library の代表 status は、全 direct verifier が accepted の場合だけ verified とする。
- detail の verification evidence は、direct verifier ごとの状態をすべて表示する。

## 8. Solution browse

### 8.1 Solutions root

```text
main
|- page header > h1 Solutions
`- section.contests
   `- ul > li > article.contest-card
```

Contest card:

- OJ
- contest title と link
- public problem count
- public solution count
- latest solved time

### 8.2 Contest

```text
main
|- page header
|  |- h1 contest title
|  |- OJ
|  `- official contest link
`- section.problems
   `- ul > li > article.problem-card
```

Problem card:

- problem title / code と link
- public solution count
- latest solved time

### 8.3 Problem

```text
main
|- page header
|  |- h1 problem title
|  |- contest / OJ
|  `- official problem link
`- section.solutions
   `- ul > li > article.solution-card
```

Solution card:

- solution name と detail link
- language
- solved time
- verification status badge
- direct public dependency count

全 browse page は公開 solution だけから生成し、Pagefind index から除外する。

## 9. Solution detail

```text
article.solution-detail
|- page header
|  |- h1 solution name
|  |- contest / problem / OJ
|  |- language / solved time
|  `- verification status badge
|- nav.in-page-navigation
|- section#source
|- section#libraries
|  |- verifies
|  `- depends on
|- optional section#verification
`- optional section#diagnostics
```

- source は repository の entry file。
- preprocess があれば、OJ 上の source そのものではないと明示する。
- verifies と direct dependencies は別 list。
- `not_configured` は verification section を省略する。
- `never` は verification section に未実行の empty-state を表示する。
- result summary は verdict、judged time、実行時間、memory、OJ link の `dl`。
- testcase detail がある場合だけ caption と column heading を持つ `table` を表示する。
- entry source 外の diagnostic location は path / line / column を出さず、共通 reason を表示する。
- article が Pagefind body。
- verification UI と navigation は全文検索本文から除外する。

## 10. Search

global search form がこの page でも唯一の form。input は URL の `q` を復元する。

```text
main
|- page header > h1 Search
|- parsed filters
|- search status / error
|- result summary
|- ol.search-results
|  `- li > article.search-result-card
|     |- h2 > detail link
|     |- type / language / status / path
|     |- match reasons
|     |- excerpt
|     |- ul.sub-results
|     `- optional more-matches link
`- nav.pagination
```

Search URL:

```text
/search/?q={query}&page={positive-integer}
```

状態:

- no query: grammar と例を表示し、全件検索しない。
- loading: `role="status"`。
- query error: `role="alert"`。
- result count update: `aria-live="polite"`。
- zero result: 専用 message。
- Pagefind load failure: query error と別 message。
- no JavaScript: `<noscript>` と Libraries / Solutions link。

結果:

- rank 順の `ol`。
- 1 page 20 file card。
- 1 card は 1 library または solution file。
- title が同じでも canonical page ID ごとに別 card とする。
- type、language、verification status、完全な公開 path を常に表示する。
- 一致理由は `Title match`、`File match`、`Symbol match` の text label。
- 同じ page の複数の一致理由は 1 card に集約し、label を重複させない。
- 1 card の symbol / line sub-result は最大 5 件。
- exact symbol sub-result を先にし、location があれば line 順にする。
- sub-result は symbol location または `L*` へ直接 link。
- 5 件を超える場合は `ほか N 件` と detail page への link を表示する。
- Previous / Next は `q` を維持する。
- 不正または範囲外 page は 1 へ canonicalize する。
- 検索 page 自体と検索 UI は Pagefind index から除外する。

## 11. Static 404

```text
main
|- breadcrumb: Home > Page not found
|- page header > h1 Page not found
|- short explanation
`- nav.recovery-navigation
   |- Home
   |- Libraries
   |- Solutions
   `- Search
```

- 共通 header、global search、footer を使う。
- requested path 表示用 JavaScript は使わない。
- redirect や類似 page 推測はしない。
- `noindex` と Pagefind index 除外を指定する。

## 12. Shared status component

```html
<span class="status-badge" data-status="verified">
  <svg aria-hidden="true">...</svg>
  <span>Verified</span>
</span>
```

Verification labels:

| `data-status` | Label | Usage |
| --- | --- | --- |
| `verified` | Verified | Current accepted result |
| `rejected` | Rejected | Current non-accepted verdict |
| `unavailable` | Unavailable | Stable OJ capability mismatch |
| `stale` | Stale | Saved result differs from current input |
| `never` | Never verified | Verify target without a result |
| `not_configured` | Verification not configured | Non-verify solution only |

Analysis labels:

| `data-status` | Label |
| --- | --- |
| `complete` | Analysis complete |
| `partial` | Analysis partial |
| `failed` | Analysis failed |

- 色だけで状態を表さない。
- icon は decorative、label は常に text で存在する。
- static badge に alert / live role を付けない。
- list card は badge、detail は必要に応じて説明 callout も表示する。
- callout は対象、理由、影響、次の link を文章で示す。
- time は RFC 3339 `datetime` を持つ `<time>`。

## 13. Shared source viewer

```html
<section id="source" aria-labelledby="source-heading">
  <h2 id="source-heading">Source</h2>
  <div class="source-toolbar" data-pagefind-ignore>...</div>
  <pre class="source-code" tabindex="0"
       aria-labelledby="source-heading"><code data-language="rust">
    <span id="L1" class="source-line" data-line="1"><a
      class="source-line-number" href="#L1" aria-label="Line 1"
      data-pagefind-ignore>1</a><span class="source-line-content">...</span></span>
  </code></pre>
</section>
```

固定 contract:

- 全行に 1-based `L*` ID と permalink。
- 空行にも source-line element。
- line number と line content は別 element。
- Shiki token は source-line-content 内だけ。
- toolbar と line number は検索対象外。line content は検索対象。
- pre は keyboard focus 可能な横 scroll region。
- MVP では wrap toggle、line range、copy button なし。
- repository link は表示内容と同じ build commit / path を指す。
- 公開可能な source は全行を同じ DOM に持ち、途中 truncate / virtualize を行わない。

## 14. Shared detail lists

Symbols:

```text
section > ul > li
|- kind text
|- code name
|- optional qualified name / signature
`- optional source location link
```

Direct dependencies / direct dependents / relations:

```text
section > ul > li
|- target title / link
|- language / path
`- relation kind または manual marker
```

Verification evidence:

```text
section > ul > li > article
|- solution link
|- status badge
|- judged time
`- OJ link
```

Diagnostics:

```text
section > ul > li
|- severity text
|- optional adapter code
|- message
`- optional source location link
```

- optional field がなければ field 自体を省略する。
- 0 件は空 list でなく状態別 empty-state message。
- symbol text は検索対象。
- dependency、verification、diagnostic UI は全文検索本文から除外。
- testcase detail だけは table。
- 長い signature、path、message は折り返せる必要がある。

## 15. Markdown body

- page header が唯一の h1。
- Markdown 内の h1 は不許可。
- 最初の Markdown heading は h2。
- heading level の飛び越しは禁止。
- h2 から h6 は level を変更せず描画する。
- heading ID は次の repository 固有規則による ASCII の `doc-*`。
  - 表示 plain text の exact UTF-8 bytes を Unicode normalization せず SHA-256 にする。
  - ASCII英数字の lowercase hint を最大48文字で作り、空なら `h` にする。
  - digest の先頭10桁を使い、`doc-{hint}-{digest}` とする。
  - 同じIDの2件目以降は`-2`、`-3`を付ける。
- raw HTML は無効化し、生成 HTML を sanitize する。
- Markdown image は未対応で、image asset pipeline を持たない。
- relative link は build 時に公開 route または repository commit URL へ変換済みの値を受け取る。
- paragraph だけの本文も許可する。

## 16. Pagefind contract

Index 対象:

- library detail article
- solution detail article
- title / file name / symbol name / qualified name / signature
- description Markdown
- source-line-content

Index 対象外:

- Home と全 browse page
- Search と 404
- navigation、breadcrumb、footer、in-page navigation
- source toolbar と line number
- dependency、verification、diagnostic UI

Filter / metadata:

- `code_lang`
- symbol `kind`
- library の language-root-relative path segment / cumulative prefix
- solution ID の segment / cumulative prefix
- verification status
- type: library / solution

変更禁止属性:

- `data-pagefind-*`
- `data-status`
- `data-line`
- `id="main-content"`
- `id="documentation"`
- detail section ID
- `id="L*"` と `href="#L*"`
- search input の `name="q"`

## 17. Design fixture checklist

最低限、次の状態を同じ component system で確認する。

- Home: 通常、全 section 空、attention 10 件超
- Libraries root: 複数言語、0 言語
- Directory: description あり / なし、子のみ、file のみ、空
- Library: verified、stale、rejected、unavailable、never
- Library analysis: complete、partial、failed、symbol 0 件
- Library data: private dependency あり、長い dependency / relation list、cycle
- Source: 1 行、空行、1000 行超、非常に長い行、Unicode、plain-text fallback
- Solution: not_configured、never、verified、rejected、stale、testcase table あり / なし
- Search: no query、loading、error、0 件、1 件、20 件、21 件、同名 page、5 sub-result 超
- Content: 長い title、長い path、長い signature、長い diagnostic、欠けた optional field
- Viewport: narrow mobile、keyboard focus、horizontal source scroll
- Base path: `/` と `/compro-env/`
- 404、rename 前 route、非公開化または削除した detail の旧 route

## 18. デザイン返却時に維持するもの

返却物は visual design と、それを表現する component / CSS 案を想定する。
次の変更が必要に見える場合は、暗黙に変更せず提案として分離する。

- route の追加・削除
- data field や status の追加・削除
- semantic element の変更
- JavaScript 必須化
- source / search contract の変更
- 公開情報の拡大

## 19. Browser security contract

- external script、CSS、font、analytics、iframe を追加しない。
- inline script、event handler、`javascript:` URL を追加しない。
- Pagefind excerpt の強調は属性なし `mark` だけ。
- query、metadata、status、diagnostic は text として描画する。
- search result link は生成済み detail route と fragment だけ。
- external link は同じ tab が既定。新規 tab なら `noopener noreferrer` を付ける。
- CSP の都合で外部 asset が必要に見える場合は、暗黙に追加せず別提案にする。
