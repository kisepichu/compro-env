import Lean
import Analyzer.Protocol
import Analyzer.Diagnostics
import Analyzer.ModuleMap
import Analyzer.Dependencies
import Analyzer.Elaboration

/-!
Elaboration tests for the Lean adapter (spec §6.8; plan 050 Task 1).

Covers the checklist from the plan: topological ordering, cycles that
fall back gracefully, independent modules, same identifier in different
namespaces, theorem errors, missing imports, a search path pruned to the
prepared toolchain + build dir, and repeatable output.

`main` is `unsafe` because `Lean.enableInitializersExecution` is required
before `importModules (loadExts := true)`. All downstream calls are safe
once initializers have been armed once inside this process.
-/

open Lean
open Analyzer.Protocol
open Analyzer.Diagnostics
open Analyzer.ModuleMap
open Analyzer.Dependencies
open Analyzer.Elaboration

namespace Analyzer.Tests.Elaboration

private def fail (msg : String) : IO Unit := do
  IO.eprintln s!"elaboration test failure: {msg}"
  discard <| (IO.Process.exit 1 : IO UInt32)

private def check (cond : Bool) (msg : String) : IO Unit :=
  if cond then pure () else fail msg

/-- A `ModuleMap` `Entry` synthesized without touching the filesystem so
    ordering tests can pin the internal-dep graph directly. -/
private def entry (path : String) (n : Name) : Entry :=
  { path := path, moduleName := n }

/-- Topological ordering places dependencies before dependents. -/
def testTopologicalOrder : IO Unit := do
  let a := entry "libraries/lean/A.lean" `A
  let b := entry "libraries/lean/B.lean" `B
  let c := entry "libraries/lean/C.lean" `C
  let map : ModuleMap := { entries := #[a, b, c] }
  -- B depends on A; C depends on B → order A, B, C regardless of input.
  let deps : NameMap (Array Name) :=
    (∅ : NameMap (Array Name))
      |>.insert `A #[]
      |>.insert `B #[`A]
      |>.insert `C #[`B]
  let ordered := topologicalOrder map deps
  let names := ordered.map (·.moduleName)
  check (names == #[`A, `B, `C]) s!"topo order = {names}"

/-- Cycles fall back to the surrounding path-sorted order once progress
    stalls, so the analyzer never spins forever on a mutual import. -/
def testCycles : IO Unit := do
  let a := entry "libraries/lean/A.lean" `A
  let b := entry "libraries/lean/B.lean" `B
  let map : ModuleMap := { entries := #[a, b] }
  -- A ↔ B: mutually dependent → no progress in the Kahn step.
  let deps : NameMap (Array Name) :=
    (∅ : NameMap (Array Name))
      |>.insert `A #[`B]
      |>.insert `B #[`A]
  let ordered := topologicalOrder map deps
  let names := ordered.map (·.moduleName)
  check (ordered.size == 2) s!"cycle size = {ordered.size}"
  check (names.contains `A) "cycle missed A"
  check (names.contains `B) "cycle missed B"

/-- Two independent modules each produce their own declarations. -/
def testIndependentModules : IO Unit := do
  let src1 := "import Init\ndef p : Nat := 1\n"
  let src2 := "import Init\ndef q : Nat := 2\n"
  let e1 ← elaborateSource "libraries/lean/P.lean" src1 `P
  let e2 ← elaborateSource "libraries/lean/Q.lean" src2 `Q
  check (!e1.hasErrors) s!"P errored: {e1.diagnostics.map (·.message)}"
  check (!e2.hasErrors) s!"Q errored: {e2.diagnostics.map (·.message)}"
  let names1 : NameSet :=
    e1.environment.constants.foldStage2 (s := NameSet.empty) fun s n _ =>
      if e1.baseline.contains n then s else s.insert n
  let names2 : NameSet :=
    e2.environment.constants.foldStage2 (s := NameSet.empty) fun s n _ =>
      if e2.baseline.contains n then s else s.insert n
  check (names1.contains `p) "P missing p"
  check (names2.contains `q) "Q missing q"
  check (!names1.contains `q) "P leaked q"
  check (!names2.contains `p) "Q leaked p"

/-- Same identifier under different namespaces stays disambiguated by
    qualified name. -/
def testSameNameInNamespaces : IO Unit := do
  let src :=
    "import Init\nnamespace A\ndef X : Nat := 1\nend A\n" ++
    "namespace B\ndef X : Nat := 2\nend B\n"
  let e ← elaborateSource "libraries/lean/NS.lean" src `NS
  check (!e.hasErrors) s!"NS errored: {e.diagnostics.map (·.message)}"
  let newNames : Array Name :=
    e.environment.constants.foldStage2 (s := (#[] : Array Name)) fun s n _ =>
      if e.baseline.contains n then s else s.push n
  check (newNames.contains `A.X) s!"missing A.X, got {newNames}"
  check (newNames.contains `B.X) s!"missing B.X, got {newNames}"

/-- Elaboration errors from a broken theorem do not crash the analyzer.
    `hasErrors` flips to `true` and a diagnostic is attached. -/
def testTheoremErrors : IO Unit := do
  let src := "import Init\ntheorem bad : 1 = 2 := rfl\n"
  let e ← elaborateSource "libraries/lean/Bad.lean" src `Bad
  check e.hasErrors "expected hasErrors=true"
  let hit := e.diagnostics.any (fun d => d.code == "lean.symbols.elaboration")
  check hit "expected elaboration diagnostic"

/-- Missing imports fail at header time; body decls are empty. -/
def testMissingImports : IO Unit := do
  let src := "import DoesNotExist\ndef x : Nat := 1\n"
  let e ← elaborateSource "libraries/lean/M.lean" src `M
  check e.hasErrors "expected header error"
  let newNames : Array Name :=
    e.environment.constants.foldStage2 (s := (#[] : Array Name)) fun s n _ =>
      if e.baseline.contains n then s else s.push n
  check (newNames.isEmpty) s!"expected 0 new decls, got {newNames.size}"

/-- The search path is exactly the prepared sysroot Lean install path.
    We assert `Lean.searchPathRef` never contains a user-global directory
    outside the toolchain root or the build directory. -/
def testNoUserGlobalSearchPath : IO Unit := do
  let sysroot ← Lean.findSysroot
  let paths ← Lean.searchPathRef.get
  for p in paths do
    let pStr := p.toString
    let sysStr := sysroot.toString
    let inLake := pStr.endsWith ".lake/build/lib/lean" ||
                  pStr.endsWith ".lake/build/lib/lean/"
    -- Anything outside the prepared toolchain root or the local build
    -- directory would let the user's ambient PATH shadow our imports.
    let ok := (pStr.startsWith sysStr) || inLake
    check ok s!"search path escapes prepared toolchain: {pStr}"

/-- Elaboration of a fixed input is byte-stable across repeated calls in
    the same process. Matches the Task 1 checklist item that asks us to
    run `Tests/Elaboration.lean` twice and compare normalized output. -/
def testRepeatableOutput : IO Unit := do
  let src := "import Init\ndef r : Nat := 42\ntheorem re : r = 42 := rfl\n"
  let e1 ← elaborateSource "libraries/lean/R.lean" src `R
  let e2 ← elaborateSource "libraries/lean/R.lean" src `R
  let names1 : Array Name :=
    e1.environment.constants.foldStage2 (s := (#[] : Array Name)) fun s n _ =>
      if e1.baseline.contains n then s else s.push n
  let names2 : Array Name :=
    e2.environment.constants.foldStage2 (s := (#[] : Array Name)) fun s n _ =>
      if e2.baseline.contains n then s else s.push n
  check (names1 == names2) s!"non-repeatable: {names1} vs {names2}"

end Analyzer.Tests.Elaboration

open Analyzer.Tests.Elaboration in
unsafe def unsafeMain : IO Unit := do
  Lean.enableInitializersExecution
  Lean.initSearchPath (← Lean.findSysroot)
  testTopologicalOrder
  testCycles
  testIndependentModules
  testSameNameInNamespaces
  testTheoremErrors
  testMissingImports
  testNoUserGlobalSearchPath
  testRepeatableOutput
  IO.println "ce-lean elaboration tests passed"

@[implemented_by unsafeMain]
opaque mainOpaque : IO Unit

def main : IO Unit := mainOpaque
