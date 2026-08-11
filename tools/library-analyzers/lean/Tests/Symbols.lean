import Lean.Data.Json
import Analyzer.Protocol
import Analyzer.Diagnostics
import Analyzer.ModuleMap
import Analyzer.Dependencies
import Analyzer.Elaboration
import Analyzer.Symbols

/-!
Symbol-extraction tests for the Lean adapter (spec §6.8; plan 050 Task 2).

Covers the plan checklist: definition, theorem, axiom, structure, class,
inductive / constructors, instance, namespace qualification, Unicode
name, generated declaration filtering, and one-based Unicode source
ranges.

The fixture-driven leg loads
`tools/library-analyzers/protocol/fixtures/lean-symbols-request.json`
(with `<REPOSITORY_ROOT>` substituted for the checked-in synthetic tree
at `./tests/fixtures/tree`), runs the analyzer, and compares the
serialized response to `lean-symbols-response.json` via canonical
`.compress` strings — key order does not affect the outcome because
`Json.obj` is an RB tree keyed by string.

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
open Analyzer.Elaboration
open Analyzer.Symbols

namespace Analyzer.Tests.Symbols

private def expectedLeanVersion : String := "4.30.0"

private def fail (msg : String) : IO Unit := do
  IO.eprintln s!"symbols test failure: {msg}"
  discard <| (IO.Process.exit 1 : IO UInt32)

private def check (cond : Bool) (msg : String) : IO Unit :=
  if cond then pure () else fail msg

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

/-- End-to-end: request fixture → analyzer → response JSON matches the
    checked-in response fixture byte-for-byte after canonical compression. -/
def testFixtureResponse : IO Unit := do
  let fixDir ← fixtureDir
  let tree ← treeDir
  let treeStr := tree.toString
  let reqRaw ← readFixture fixDir "lean-symbols-request.json"
  let reqSubst := reqRaw.replace "<REPOSITORY_ROOT>" treeStr
  let req ← match Analyzer.Protocol.parseRequest reqSubst with
    | .ok r    => pure r
    | .error e => do fail s!"request rejected: [{e.code}] {e.message}"; pure default
  let map ← match Analyzer.ModuleMap.buildModuleMap req with
    | .ok m    => pure m
    | .error d => do fail s!"module map rejected: [{d.code}] {d.message}"; pure default
  let (libAnalyses, solAnalyses) ← Analyzer.Dependencies.analyzeRequest map req
  let libPairs : Array (Entry × TargetAnalysis) :=
    (req.libraries.zip libAnalyses).map fun ⟨lib, a⟩ =>
      match map.entries.find? (fun e => e.path == lib.path) with
      | some e => (e, a)
      | none   => ((⟨lib.path, .anonymous⟩ : Entry), a)
  let depsMap := Analyzer.Elaboration.internalDepsMap map libPairs
  let elaborated ← Analyzer.Elaboration.elaborateTargets req map depsMap
  let libsJson : Array Json := (req.libraries.zip libAnalyses).map fun ⟨lib, a⟩ =>
    let matched := elaborated.find? (·.path == lib.path)
    let sym : SymbolAnalysis := match matched with
      | some t => extractSymbols t
      | none   => { state := "partial", symbols := #[] }
    let elabDiags : Array Diagnostic := match matched with
      | some t => t.diagnostics
      | none   => #[]
    let a' := { a with diagnostics := a.diagnostics ++ elabDiags }
    Analyzer.Dependencies.libraryJsonWithSymbols lib.path a'
      (Analyzer.Symbols.symbolAnalysisToJson sym)
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
  let expRaw ← readFixture fixDir "lean-symbols-response.json"
  let expected ← match Json.parse expRaw with
    | .ok j    => pure j
    | .error e => do fail s!"expected fixture: {e}"; pure Json.null
  let actualStr := actual.compress
  let expectedStr := expected.compress
  if actualStr != expectedStr then
    IO.eprintln s!"--- expected ---\n{expected.pretty}"
    IO.eprintln s!"--- actual ---\n{actual.pretty}"
    fail "response fixture drift"

private def elabSymbols (path source : String) (modName : Name) :
    IO SymbolAnalysis := do
  let target ← elaborateSource path source modName
  return extractSymbols target

/-- `def` / `theorem` / `axiom` / `opaque` / `abbrev` all project to
    their own stable kind tokens. -/
def testBasicKinds : IO Unit := do
  let src := "import Init\ndef d : Nat := 1\ntheorem t : d = 1 := rfl\naxiom a : Nat\nopaque o : Nat\nabbrev A := Nat\n"
  let a ← elabSymbols "libraries/lean/Basic.lean" src `Basic
  check (a.state == "complete") s!"state = {a.state}"
  let kindOf : String → Option String := fun name =>
    (a.symbols.find? (·.name == name)).map (·.kind)
  check (kindOf "d" == some "def")     s!"d kind = {kindOf "d"}"
  check (kindOf "t" == some "theorem") s!"t kind = {kindOf "t"}"
  check (kindOf "a" == some "axiom")   s!"a kind = {kindOf "a"}"
  check (kindOf "o" == some "opaque")  s!"o kind = {kindOf "o"}"
  check (kindOf "A" == some "abbrev")  s!"A kind = {kindOf "A"}"

/-- Structures emit the struct itself, its constructor, and each field
    (plain `defnInfo` projections at the field source range). -/
def testStructure : IO Unit := do
  let src := "import Init\nstructure Point where\n  x : Nat\n  y : Nat\n"
  let a ← elabSymbols "libraries/lean/S.lean" src `S
  let byName : String → Option Symbol := fun n => a.symbols.find? (·.name == n)
  check ((byName "Point").map (·.kind) == some "structure")
    s!"Point kind = {(byName "Point").map (·.kind)}"
  check ((byName "x").map (·.kind) == some "field")
    s!"x kind = {(byName "x").map (·.kind)}"
  check ((byName "y").map (·.kind) == some "field")
    s!"y kind = {(byName "y").map (·.kind)}"
  let mk := a.symbols.find? (fun s => s.qualifiedName == some "Point.mk")
  check (mk.map (·.kind) == some "constructor")
    s!"Point.mk kind = {mk.map (·.kind)}"

/-- Classes are structures with `isClass = true`; instances become
    `instance` — not `def` — because `isInstanceCore` fires. -/
def testClassAndInstance : IO Unit := do
  let src :=
    "import Init\nclass HasFoo (α : Type) where\n  foo : α\ninstance : HasFoo Nat where\n  foo := 0\n"
  let a ← elabSymbols "libraries/lean/C.lean" src `C
  let byName : String → Option Symbol := fun n => a.symbols.find? (·.name == n)
  check ((byName "HasFoo").map (·.kind) == some "class")
    s!"HasFoo kind = {(byName "HasFoo").map (·.kind)}"
  check ((byName "foo").map (·.kind) == some "field")
    s!"foo kind = {(byName "foo").map (·.kind)}"
  let inst := a.symbols.find? (fun s => s.kind == "instance")
  check inst.isSome "expected at least one instance"

/-- Inductive types and their constructors carry the right kinds even
    when the type has no fields (non-structural inductive). -/
def testInductiveAndConstructors : IO Unit := do
  let src :=
    "import Init\ninductive Tree where\n  | leaf\n  | node (l r : Tree)\n"
  let a ← elabSymbols "libraries/lean/I.lean" src `I
  let byName : String → Option Symbol := fun n => a.symbols.find? (·.name == n)
  check ((byName "Tree").map (·.kind) == some "inductive")
    s!"Tree kind = {(byName "Tree").map (·.kind)}"
  let leaf := a.symbols.find? (fun s => s.qualifiedName == some "Tree.leaf")
  check (leaf.map (·.kind) == some "constructor") "Tree.leaf missing"
  let node := a.symbols.find? (fun s => s.qualifiedName == some "Tree.node")
  check (node.map (·.kind) == some "constructor") "Tree.node missing"

/-- Namespaces surface in `qualified_name`; `search_names` carry both
    the short display name and the dotted qualified variant. -/
def testNamespaceQualification : IO Unit := do
  let src :=
    "import Init\nnamespace Outer\ndef inNS : Nat := 1\nnamespace Inner\ndef deep : Nat := 2\nend Inner\nend Outer\n"
  let a ← elabSymbols "libraries/lean/N.lean" src `N
  let inNS := a.symbols.find? (fun s => s.name == "inNS")
  check inNS.isSome "inNS missing"
  match inNS with
  | some s =>
    check (s.qualifiedName == some "Outer.inNS") s!"inNS qn = {s.qualifiedName}"
    check (s.searchNames == #["inNS", "Outer.inNS"])
      s!"inNS search = {s.searchNames}"
  | none => pure ()
  let deep := a.symbols.find? (fun s => s.name == "deep")
  check ((deep.bind (·.qualifiedName)) == some "Outer.Inner.deep")
    s!"deep qn = {deep.bind (·.qualifiedName)}"

/-- Kernel-generated declarations (`recOn`, `noConfusion`, `sizeOf_spec`,
    …) never carry a `declRange`, so they are silently filtered out. -/
def testGeneratedDeclarationsFiltered : IO Unit := do
  let src :=
    "import Init\ninductive Color where\n  | red\n  | green\n"
  let a ← elabSymbols "libraries/lean/G.lean" src `G
  for s in a.symbols do
    let n := s.qualifiedName.getD s.name
    check (!(n.endsWith ".recOn" || n.endsWith ".rec" ||
             n.endsWith ".casesOn" || n.endsWith ".noConfusion" ||
             n.endsWith ".ctorIdx" || n.endsWith ".sizeOf_spec"))
      s!"leaked generated decl: {n}"

/-- Body elaboration failure (bad `rfl`) keeps the symbols we managed
    to project but drops the analysis state to `partial` and attaches a
    stable `lean.symbols.elaboration` error diagnostic. -/
def testTheoremErrorPartial : IO Unit := do
  let src :=
    "import Init\ndef ok : Nat := 1\ntheorem bad : 1 = 2 := rfl\n"
  let target ← elaborateSource "libraries/lean/B.lean" src `B
  let a := extractSymbols target
  check (a.state == "partial") s!"state = {a.state}"
  let ok := a.symbols.find? (·.name == "ok")
  check ok.isSome "expected `ok` symbol"
  let elabHit := target.diagnostics.any
    (fun d => d.code == "lean.symbols.elaboration" && d.severity == .error)
  check elabHit "expected lean.symbols.elaboration diagnostic"

/-- Unicode identifiers survive intact (Greek letters here — full CJK
    would need `«…»` escaping which Lean's own lexer requires). -/
def testUnicodeIdentifier : IO Unit := do
  let src := "import Init\ndef αβγ : Nat := 3\n"
  let a ← elabSymbols "libraries/lean/U.lean" src `U
  check (a.state == "complete") s!"state = {a.state}"
  let hit := a.symbols.find? (·.name == "αβγ")
  check hit.isSome "expected Unicode-named def"
  match hit with
  | some s =>
    match s.location.bind (·.start) with
    | some p =>
      check (p.line == 2) s!"unicode start line = {p.line}"
      check (p.column == some 1) s!"unicode start col = {p.column}"
    | none => fail "unicode symbol missing location"
  | none => pure ()

end Analyzer.Tests.Symbols

open Analyzer.Tests.Symbols in
unsafe def unsafeMain : IO Unit := do
  Lean.enableInitializersExecution
  Lean.initSearchPath (← Lean.findSysroot)
  testBasicKinds
  testStructure
  testClassAndInstance
  testInductiveAndConstructors
  testNamespaceQualification
  testGeneratedDeclarationsFiltered
  testTheoremErrorPartial
  testUnicodeIdentifier
  testFixtureResponse
  IO.println "ce-lean symbols tests passed"

@[implemented_by unsafeMain]
opaque mainOpaque : IO Unit

def main : IO Unit := mainOpaque
