import Lean.Data.Json
import Analyzer.Protocol
import Analyzer.Diagnostics
import Analyzer.ModuleMap
import Analyzer.Dependencies

/-!
Dependency-analysis tests for the Lean adapter (spec §6.8; plan 049 Task 2).

Covers the plan checklist: multiple imports, internal / external / unresolved
classification, header cycles surfaced as separate edges, missing modules
under a managed root, malformed headers, header-only comments, Unicode
before the `import` keyword, deduplication, and one-based Unicode-scalar
source spans.

The fixture-driven leg loads
`tools/library-analyzers/protocol/fixtures/lean-dependencies-request.json`
(with `<REPOSITORY_ROOT>` substituted for the checked-in synthetic tree at
`./tests/fixtures/tree`), runs the analyzer, and compares the serialized
response to `lean-dependencies-response.json` via canonical `.compress`
strings — key order does not affect the outcome because `Json.obj` is an
RB tree keyed by string. Any semantic drift surfaces as a mismatch.

Environment overrides mirror `Tests/Handshake.lean`:
* `CE_LEAN_FIXTURE_DIR` — protocol fixtures directory (default
  `../protocol/fixtures`).
* `CE_LEAN_TREE_DIR`    — synthetic fixture tree root (default
  `./tests/fixtures/tree`).
-/

open Lean
open Analyzer.Protocol
open Analyzer.Diagnostics
open Analyzer.ModuleMap
open Analyzer.Dependencies

namespace Analyzer.Tests.Dependencies

private def expectedLeanVersion : String := "4.30.0"

private def fail (msg : String) : IO α := do
  IO.eprintln s!"dependencies test failure: {msg}"
  IO.Process.exit 1

private def check (cond : Bool) (msg : String) : IO Unit :=
  if cond then pure () else discard (fail msg)

private def fixtureDir : IO System.FilePath := do
  match ← IO.getEnv "CE_LEAN_FIXTURE_DIR" with
  | some p => return System.FilePath.mk p
  | none   => return System.FilePath.mk "../protocol/fixtures"

private def treeDir : IO System.FilePath := do
  match ← IO.getEnv "CE_LEAN_TREE_DIR" with
  | some p => return System.FilePath.mk p
  | none   => return System.FilePath.mk "./tests/fixtures/tree"

private def readFixture (dir : System.FilePath) (name : String) : IO String :=
  IO.FS.readFile (dir / name)

/-- Analyzer output for the checked-in fixture request against the checked-in
    synthetic tree matches the checked-in response fixture byte-for-byte
    after canonical compression. -/
def testFixtureResponse : IO Unit := do
  let fixDir ← fixtureDir
  let tree ← treeDir
  let treeStr := tree.toString
  let reqRaw ← readFixture fixDir "lean-dependencies-request.json"
  let reqSubst := reqRaw.replace "<REPOSITORY_ROOT>" treeStr
  let req ← match Analyzer.Protocol.parseRequest reqSubst with
    | .ok r    => pure r
    | .error e => fail s!"request rejected: [{e.code}] {e.message}"
  let map ← match Analyzer.ModuleMap.buildModuleMap req with
    | .ok m    => pure m
    | .error d => fail s!"module map rejected: [{d.code}] {d.message}"
  let (libAnalyses, solAnalyses) ← Analyzer.Dependencies.analyzeRequest map req
  let libsJson : Array Json := (req.libraries.zip libAnalyses).map
    (fun ⟨lib, a⟩ => Analyzer.Dependencies.libraryJson lib.path a)
  let solsJson : Array Json := (req.solutions.zip solAnalyses).map
    (fun ⟨sol, a⟩ => Analyzer.Dependencies.solutionJson sol.id a)
  let adapter := Analyzer.Protocol.handshakeAdapter expectedLeanVersion
  let actual := Json.mkObj [
    ("schema_version",
      Json.num (JsonNumber.fromNat Analyzer.Protocol.schemaVersion)),
    ("adapter", Analyzer.Protocol.adapterToJson adapter),
    ("libraries", Json.arr libsJson),
    ("solutions", Json.arr solsJson)
  ]
  let expRaw ← readFixture fixDir "lean-dependencies-response.json"
  let expected ← match Json.parse expRaw with
    | .ok j    => pure j
    | .error e => fail s!"expected fixture: {e}"
  let actualStr := actual.compress
  let expectedStr := expected.compress
  if actualStr != expectedStr then
    IO.eprintln s!"--- expected ---\n{expected.pretty}"
    IO.eprintln s!"--- actual ---\n{actual.pretty}"
    discard (fail "response fixture drift")

private def libReq (root : String) (paths : List String) : AnalysisRequest :=
  { schemaVersion  := 1
    repositoryRoot := root
    language       := "lean"
    libraries      := paths.toArray.map
                        (fun p => ({ path := p } : LibraryTarget))
    solutions      := #[]
  }

/-- Import statements that name the same module twice collapse to one
    edge, retaining the first-seen span. -/
def testDeduplication : IO Unit := do
  let req := libReq "." ["libraries/lean/Dedup.lean",
                          "libraries/lean/A.lean",
                          "libraries/lean/A/B.lean"]
  let map ← match Analyzer.ModuleMap.buildModuleMap req with
    | .ok m    => pure m
    | .error d => fail s!"module map rejected: [{d.code}] {d.message}"
  let source := "import A\nimport A.B\nimport A\n"
  let a ← Analyzer.Dependencies.analyzeSource map "libraries/lean/Dedup.lean" source
  check (a.dependencies.size == 2)
    s!"expected 2 deduped deps, got {a.dependencies.size}"
  match a.dependencies[0]! with
  | .internal p loc =>
    check (p == "libraries/lean/A.lean")
      s!"first dep path = {p}"
    match loc.start with
    | some pos => check (pos.line == 1) s!"first dep line = {pos.line}"
    | none     => discard (fail "first dep missing start")
  | _ => discard (fail "first dep not internal")
  match a.dependencies[1]! with
  | .internal p _ =>
    check (p == "libraries/lean/A/B.lean") s!"second dep path = {p}"
  | _ => discard (fail "second dep not internal")

/-- Position conversion counts Unicode scalar values, not UTF-8 bytes, so
    the `import` after a block comment containing a multi-byte character
    lands at column 8 (not 10 as byte-counting would give). -/
def testUnicodeColumnConversion : IO Unit := do
  let req := libReq "." ["libraries/lean/A.lean", "libraries/lean/A/B.lean"]
  let map ← match Analyzer.ModuleMap.buildModuleMap req with
    | .ok m    => pure m
    | .error d => fail s!"module map rejected: [{d.code}] {d.message}"
  let source := "-- ascii comment\n/- 日 -/import A\n"
  let a ← Analyzer.Dependencies.analyzeSource map "libraries/lean/A/B.lean" source
  check (a.dependencies.size == 1)
    s!"expected 1 dep, got {a.dependencies.size}"
  match a.dependencies[0]! with
  | .internal _ loc =>
    match loc.start, loc.«end» with
    | some s, some e =>
      check (s.line == 2)         s!"unicode start line = {s.line}"
      check (s.column == some 8)  s!"unicode start col = {s.column}"
      check (e.line == 2)         s!"unicode end line = {e.line}"
      check (e.column == some 16) s!"unicode end col = {e.column}"
    | _, _ => discard (fail "unicode dep missing start/end")
  | _ => discard (fail "unicode dep not internal")

/-- A header parse error yields state `failed` when no imports survive and
    at least one `lean.dependencies.header_parse` diagnostic is attached. -/
def testMalformedHeader : IO Unit := do
  let req := libReq "." ["libraries/lean/Standalone.lean"]
  let map ← match Analyzer.ModuleMap.buildModuleMap req with
    | .ok m    => pure m
    | .error d => fail s!"module map rejected: [{d.code}] {d.message}"
  let a ← Analyzer.Dependencies.analyzeSource map "libraries/lean/Standalone.lean"
    "import 123\n"
  check (a.state == "failed") s!"malformed state = {a.state}"
  check (a.dependencies.isEmpty)
    s!"malformed emitted {a.dependencies.size} deps"
  let hit := a.diagnostics.any
    (fun d => d.code == "lean.dependencies.header_parse")
  check hit "expected header_parse diagnostic"

end Analyzer.Tests.Dependencies

def main : IO Unit := do
  Analyzer.Tests.Dependencies.testFixtureResponse
  Analyzer.Tests.Dependencies.testDeduplication
  Analyzer.Tests.Dependencies.testUnicodeColumnConversion
  Analyzer.Tests.Dependencies.testMalformedHeader
  IO.println "ce-lean dependencies tests passed"
