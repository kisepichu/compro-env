import Lean.Data.Name
import Analyzer.Protocol
import Analyzer.Diagnostics
import Analyzer.ModuleMap

/-!
Module-map tests for the Lean adapter (spec §6.8; plan 049 Task 1).

Exercises `Analyzer.ModuleMap.buildModuleMap` and `moduleForPath` against
the checklist items from the plan: nested modules, `Main.lean`, Unicode
names, duplicate module ownership, invalid path components, repository
escapes, non-`.lean` files, and stable UTF-8 byte ordering.

The harness mirrors `Tests/Handshake.lean`: an `IO` runner that exits with
status 1 on the first assertion failure, so `lake env lean Tests/ModuleMap.lean`
fails loudly instead of silently.
-/

open Lean
open Analyzer.Diagnostics
open Analyzer.Protocol
open Analyzer.ModuleMap

namespace Analyzer.Tests.ModuleMap

private def fail (msg : String) : IO Unit := do
  IO.eprintln s!"module map test failure: {msg}"
  discard <| (IO.Process.exit 1 : IO UInt32)

private def check (cond : Bool) (msg : String) : IO Unit :=
  if cond then pure () else fail msg

/-- Build an `AnalysisRequest` populated with only library targets. -/
private def libReq (paths : List String) : AnalysisRequest :=
  { schemaVersion  := 1
    repositoryRoot := "."
    language       := "lean"
    libraries      := paths.toArray.map
                        (fun p => ({ path := p } : LibraryTarget))
    solutions      := #[]
  }

/-- Build an `AnalysisRequest` with a single solution target. -/
private def solReq (id root entry : String) : AnalysisRequest :=
  { schemaVersion  := 1
    repositoryRoot := "."
    language       := "lean"
    libraries      := #[]
    solutions      := #[({ id, root, entry } : SolutionTarget)]
  }

/-- Compose a module name from segments left-to-right. -/
private def mkName (parts : List String) : Name :=
  parts.foldl (fun acc p => acc.mkStr p) .anonymous

/-- Nested library paths become dotted Lean module names. -/
def testNestedModule : IO Unit := do
  let path := "libraries/lean/Foo/Bar.lean"
  match buildModuleMap (libReq [path]) with
  | .error d => (fail s!"nested module rejected: [{d.code}] {d.message}")
  | .ok m =>
    match moduleForPath m path with
    | some name =>
      check (name == mkName ["Foo", "Bar"])
        s!"expected Foo.Bar, got {name}"
    | none => (fail s!"path not in map: {path}")

/-- A top-level `Main.lean` maps to the bare module `Main` without any
    special casing. -/
def testMainLean : IO Unit := do
  let path := "libraries/lean/Main.lean"
  match buildModuleMap (libReq [path]) with
  | .error d => (fail s!"Main.lean rejected: [{d.code}] {d.message}")
  | .ok m =>
    match moduleForPath m path with
    | some name =>
      check (name == mkName ["Main"]) s!"expected Main, got {name}"
    | none => (fail s!"path not in map: {path}")

/-- Unicode component names round-trip through the mapping. -/
def testUnicodeName : IO Unit := do
  let path := "libraries/lean/日本語.lean"
  match buildModuleMap (libReq [path]) with
  | .error d => (fail s!"unicode rejected: [{d.code}] {d.message}")
  | .ok m =>
    match moduleForPath m path with
    | some name =>
      check (name == mkName ["日本語"]) s!"expected 日本語, got {name}"
    | none => (fail s!"path not in map: {path}")

/-- Two distinct paths that would both resolve to module `Foo` are
    rejected before any parsing happens. -/
def testDuplicateModuleOwnership : IO Unit := do
  let req := libReq ["libraries/lean/Foo.lean", "other/lean/Foo.lean"]
  match buildModuleMap req with
  | .ok _ => (fail "duplicate module ownership was accepted")
  | .error d =>
    check (d.code == "lean.module_map.duplicate_owner")
      s!"expected duplicate_owner, got [{d.code}] {d.message}"

/-- The same path listed twice is a duplicate owner too. -/
def testDuplicatePath : IO Unit := do
  let req := libReq ["libraries/lean/Foo.lean", "libraries/lean/Foo.lean"]
  match buildModuleMap req with
  | .ok _ => (fail "duplicate path was accepted")
  | .error d =>
    check (d.code == "lean.module_map.duplicate_owner")
      s!"expected duplicate_owner, got [{d.code}] {d.message}"

/-- A path component that starts with an ASCII digit is not a valid Lean
    identifier component. -/
def testInvalidComponentLeadingDigit : IO Unit := do
  match buildModuleMap (libReq ["libraries/lean/1Bad.lean"]) with
  | .ok _ => (fail "digit-leading component was accepted")
  | .error d =>
    check (d.code == "lean.module_map.invalid_component")
      s!"expected invalid_component, got [{d.code}] {d.message}"

/-- A path component containing `.` cannot be a Lean identifier component
    because `.` is the module-name separator. -/
def testInvalidComponentDot : IO Unit := do
  match buildModuleMap (libReq ["libraries/lean/Foo.Bar.lean"]) with
  | .ok _ => (fail "dot-in-component was accepted")
  | .error d =>
    check (d.code == "lean.module_map.invalid_component")
      s!"expected invalid_component, got [{d.code}] {d.message}"

/-- Empty components (double-slash) are invalid identifier components. -/
def testEmptyComponent : IO Unit := do
  match buildModuleMap (libReq ["libraries/lean/Foo//Bar.lean"]) with
  | .ok _ => (fail "empty component was accepted")
  | .error d =>
    check (d.code == "lean.module_map.invalid_component")
      s!"expected invalid_component, got [{d.code}] {d.message}"

/-- `..` segments would escape the repository root. -/
def testRepositoryEscapeParent : IO Unit := do
  match buildModuleMap (libReq ["libraries/../secret.lean"]) with
  | .ok _ => (fail ".. escape was accepted")
  | .error d =>
    check (d.code == "lean.module_map.repository_escape")
      s!"expected repository_escape, got [{d.code}] {d.message}"

/-- Absolute paths escape the repository root. -/
def testRepositoryEscapeAbsolute : IO Unit := do
  match buildModuleMap (libReq ["/etc/passwd.lean"]) with
  | .ok _ => (fail "absolute path was accepted")
  | .error d =>
    check (d.code == "lean.module_map.repository_escape")
      s!"expected repository_escape, got [{d.code}] {d.message}"

/-- Files without a `.lean` suffix are not managed sources. -/
def testNotLean : IO Unit := do
  match buildModuleMap (libReq ["libraries/lean/Foo.txt"]) with
  | .ok _ => (fail "non-.lean path was accepted")
  | .error d =>
    check (d.code == "lean.module_map.not_lean")
      s!"expected not_lean, got [{d.code}] {d.message}"

/-- Given scrambled input, entries are emitted in UTF-8 byte order. The
    ASCII letters here (`A` < `M` < `Z`) are all below the Japanese
    codepoints, which are in turn below the `..` synthetic paths -- so the
    expected order is Apple, Mango, Zebra, 日本語. -/
def testStablePathByteOrdering : IO Unit := do
  let paths :=
    ["libraries/lean/Zebra.lean",
     "libraries/lean/日本語.lean",
     "libraries/lean/Apple.lean",
     "libraries/lean/Mango.lean"]
  match buildModuleMap (libReq paths) with
  | .error d =>
    (fail s!"stable-sort case rejected: [{d.code}] {d.message}")
  | .ok m =>
    let ordered := m.entries.toList.map (·.path)
    let expected :=
      ["libraries/lean/Apple.lean",
       "libraries/lean/Mango.lean",
       "libraries/lean/Zebra.lean",
       "libraries/lean/日本語.lean"]
    check (ordered == expected)
      s!"expected {expected}, got {ordered}"

/-- A solution's entry file is looked up via `<root>/<entry>` and gets a
    module name derived by the same rule as libraries. -/
def testSolutionEntry : IO Unit := do
  let req := solReq "abc999/a/main" "solutions/abc999/a/main" "Main.lean"
  match buildModuleMap req with
  | .error d => (fail s!"solution rejected: [{d.code}] {d.message}")
  | .ok m =>
    let key := "solutions/abc999/a/main/Main.lean"
    match moduleForPath m key with
    | some _ => pure ()
    | none   => (fail s!"solution entry path not in map: {key}")

/-- Paths not in the map return `none`, not a fabricated module name. -/
def testUnmanagedPathIsNone : IO Unit := do
  match buildModuleMap (libReq ["libraries/lean/Foo.lean"]) with
  | .error d => (fail s!"unexpected error: [{d.code}] {d.message}")
  | .ok m =>
    match moduleForPath m "libraries/lean/Other.lean" with
    | none   => pure ()
    | some n => (fail s!"unmanaged path returned {n}")

end Analyzer.Tests.ModuleMap

def main : IO Unit := do
  Analyzer.Tests.ModuleMap.testNestedModule
  Analyzer.Tests.ModuleMap.testMainLean
  Analyzer.Tests.ModuleMap.testUnicodeName
  Analyzer.Tests.ModuleMap.testDuplicateModuleOwnership
  Analyzer.Tests.ModuleMap.testDuplicatePath
  Analyzer.Tests.ModuleMap.testInvalidComponentLeadingDigit
  Analyzer.Tests.ModuleMap.testInvalidComponentDot
  Analyzer.Tests.ModuleMap.testEmptyComponent
  Analyzer.Tests.ModuleMap.testRepositoryEscapeParent
  Analyzer.Tests.ModuleMap.testRepositoryEscapeAbsolute
  Analyzer.Tests.ModuleMap.testNotLean
  Analyzer.Tests.ModuleMap.testStablePathByteOrdering
  Analyzer.Tests.ModuleMap.testSolutionEntry
  Analyzer.Tests.ModuleMap.testUnmanagedPathIsNone
  IO.println "ce-lean module map tests passed"
