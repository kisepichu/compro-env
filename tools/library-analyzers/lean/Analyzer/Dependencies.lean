import Lean.Data.Json
import Lean.Data.Name
import Lean.Data.NameMap
import Lean.Data.NameSet
import Lean.Elab.Import
import Lean.Parser.Module
import Analyzer.Protocol
import Analyzer.Diagnostics
import Analyzer.ModuleMap

/-!
Direct-dependency analysis for the ce-lean adapter (spec §6.8; plan 049 Task 2).

Each target — library `.lean` file or solution entry — is parsed with
`Lean.Parser.parseHeader`, and every header import is emitted as one
`Dependency` edge with a source range converted from Lean's byte offsets
to 1-based line / Unicode-scalar-value column. Only header imports count;
the ambient prelude, `open`, `include`, and theorem bodies never do.

Classification rules (spec §6.8; plan 049 constraints):

* `internal` when the import name matches an `Entry.moduleName` in the
  built `ModuleMap`.
* `unresolved` when the import name's root component matches some managed
  entry's root but no entry has that exact name — a "managed root but not
  in the manifest" edge. The key format is `missing:<Name>` and the
  display carries the literal `import <Name>` for operators.
* `external` otherwise. The name field is the dotted module name.

Imports are deduplicated by module `Name`, keeping the earliest source
span. The per-target `state` is:

* `failed` when the header parse produced errors AND no imports survived
  extraction;
* `partial` when the header parse produced errors OR at least one edge is
  `unresolved`;
* `complete` when every header import is safely classified with no parse
  errors.

Symbol analysis stays `partial` with a stable `lean.symbols.deferred`
info diagnostic per library, per plan 049 constraint: real symbols land
in plan 050. Solutions do not carry a symbol block so the deferred
diagnostic is only attached to library targets.
-/

namespace Analyzer.Dependencies

open Lean
open Analyzer.Protocol
open Analyzer.Diagnostics
open Analyzer.ModuleMap

/-- Classified dependency edge for one header import. Mirrors the Rust
    `Dependency` enum in `library-adapter-protocol`. -/
inductive Dependency where
  | internal   (path : String) (location : Location)
  | external   (name : String) (location : Location)
  | unresolved (key : String) (display : String) (location : Location)
  deriving Repr

/-- Per-target analysis payload. Diagnostics collected while parsing this
    target are attached; per-request adapter identity and toolchain live
    in the response envelope. -/
structure TargetAnalysis where
  state        : String
  dependencies : Array Dependency
  diagnostics  : Array Diagnostic
  deriving Repr

private inductive ClassifyResult where
  | internal (entry : Entry)
  | external
  | unresolved

/-- Classify one imported module name against a built `ModuleMap`.

    Exact `Entry.moduleName` match → `internal`. Otherwise, if the import
    shares its root component with any managed entry it is `unresolved`
    (managed root but no manifest match); everything else is `external`. -/
private def classifyImport (map : ModuleMap) (modName : Name) : ClassifyResult :=
  match map.entries.find? (fun e => e.moduleName == modName) with
  | some entry => .internal entry
  | none =>
    let root := modName.getRoot
    let managedRoot := map.entries.any (fun e => e.moduleName.getRoot == root)
    if managedRoot then .unresolved else .external

/-- Convert a byte offset in `source` to a 1-based line / 1-based Unicode
    scalar-value column. Newlines advance `line` and reset `column` to 1
    for the following character. Positions past the source end saturate
    at the last observed line/column. -/
private def bytePosToPosition (source : String) (pos : String.Pos) : Position := Id.run do
  let mut line : Nat := 1
  let mut col  : Nat := 1
  let mut i : String.Pos := 0
  let endPos := source.endPos
  while i < pos && i < endPos do
    let c := source.get i
    if c == '\n' then
      line := line + 1
      col := 1
    else
      col := col + 1
    i := source.next i
  return { line, column := some col }

/-- Location for a syntax node inside `source`, with `path` set to the
    caller-supplied repo-relative POSIX path. Returns `none` when the node
    has no source range (missing / synthesized syntax). -/
private def syntaxLocation (source : String) (repoRelPath : String) (stx : Syntax) :
    Option Location := do
  let startPos ← stx.getPos?
  let start := bytePosToPosition source startPos
  let endOpt := stx.getTailPos?.map (bytePosToPosition source)
  return { path := repoRelPath, start := some start, «end» := endOpt }

/-- Default line-1 column-1 location used when a syntax node has no span. -/
private def fallbackLocation (repoRelPath : String) : Location :=
  { path := repoRelPath, start := some { line := 1, column := some 1 } }

/-- Direct children of the header's `many import` slot, or `#[]` when the
    header shape is unexpected. Each child is one `Lean.Parser.Module.import`
    syntax node. -/
private def headerImportSyntaxes (header : Syntax) : Array Syntax :=
  match header.getArg 1 with
  | .node _ _ children => children
  | _ => #[]

private def stableParseErrorMessage : String :=
  "header parse produced errors; header may be malformed"

/-- Analyze one target's header from an in-memory `source` string. Splits
    the on-disk read out of `analyzeTarget` so unit tests can exercise
    header semantics (dedup, cycles, malformed headers, Unicode spans)
    without touching the filesystem. -/
def analyzeSource
    (map : ModuleMap) (repoRelPath : String) (source : String) :
    IO TargetAnalysis := do
  let input := Parser.mkInputContext source repoRelPath
  let parsed ← Parser.parseHeader input
  let headerStx := parsed.1
  let msgs := parsed.2.2
  let imports := Elab.headerToImports headerStx
  let importNodes := headerImportSyntaxes headerStx
  let mut deps : Array Dependency := #[]
  let mut seen : NameSet := {}
  let mut hasUnresolved := false
  for i in [0:imports.size] do
    let imp := imports[i]!
    if imp.module.isAnonymous then
      continue
    if seen.contains imp.module then
      continue
    seen := seen.insert imp.module
    let loc : Location :=
      match importNodes[i]? with
      | some stx =>
        (syntaxLocation source repoRelPath stx).getD (fallbackLocation repoRelPath)
      | none => fallbackLocation repoRelPath
    match classifyImport map imp.module with
    | .internal entry =>
      deps := deps.push (.internal entry.path loc)
    | .external =>
      deps := deps.push (.external imp.module.toString loc)
    | .unresolved =>
      hasUnresolved := true
      let disp := imp.module.toString
      deps := deps.push (.unresolved s!"missing:{disp}" s!"import {disp}" loc)
  let hasParseError := msgs.hasErrors
  let mut diagnostics : Array Diagnostic := #[]
  if hasParseError then
    diagnostics := diagnostics.push {
      severity := .error
      code := "lean.dependencies.header_parse"
      message := stableParseErrorMessage
      location := some (fallbackLocation repoRelPath)
    }
  let state :=
    if hasParseError && deps.isEmpty then "failed"
    else if hasParseError || hasUnresolved then "partial"
    else "complete"
  return { state, dependencies := deps, diagnostics }

/-- Analyze the header of one target and return its dependency edges.

    Reads `<repositoryRoot>/<repoRelPath>` from disk and delegates to
    `analyzeSource`. When the file cannot be read, emits a
    `lean.dependencies.read` error diagnostic and returns state `failed`. -/
def analyzeTarget
    (map : ModuleMap) (repositoryRoot : String) (repoRelPath : String) :
    IO TargetAnalysis := do
  let absPath : System.FilePath := System.FilePath.mk repositoryRoot / repoRelPath
  let sourceExists ← absPath.pathExists
  if !sourceExists then
    return {
      state := "failed"
      dependencies := #[]
      diagnostics := #[{
        severity := .error
        code := "lean.dependencies.read"
        message := s!"source file not found: {repoRelPath}"
        location := some (fallbackLocation repoRelPath)
      }]
    }
  let source ← IO.FS.readFile absPath
  analyzeSource map repoRelPath source

/-- Join a solution's `root` and `entry` into one repo-relative POSIX path
    (tolerates a trailing `/` on `root`). Mirrors the same rule
    `ModuleMap.buildModuleMap` uses. -/
private def joinRootEntry (root entry : String) : String :=
  if root.isEmpty then entry
  else if root.endsWith "/" then root ++ entry
  else s!"{root}/{entry}"

/-- Attach the plan-050 stable placeholder diagnostic to a library target's
    diagnostics array. Solutions never carry it (they have no symbol
    analysis block). -/
private def withSymbolDeferred (a : TargetAnalysis) : TargetAnalysis :=
  { a with diagnostics := a.diagnostics.push {
      severity := .info
      code := "lean.symbols.deferred"
      message := "Lean symbol extraction is deferred to plan 050."
    } }

/-- Analyze every target in `request` under the supplied `map`. Libraries
    are emitted first in request order, then solutions in request order —
    the same order the response envelope's `libraries` / `solutions`
    arrays require. -/
def analyzeRequest
    (map : ModuleMap) (request : AnalysisRequest) :
    IO (Array TargetAnalysis × Array TargetAnalysis) := do
  let mut libs : Array TargetAnalysis := #[]
  for lib in request.libraries do
    let a ← analyzeTarget map request.repositoryRoot lib.path
    libs := libs.push (withSymbolDeferred a)
  let mut sols : Array TargetAnalysis := #[]
  for sol in request.solutions do
    let path := joinRootEntry sol.root sol.entry
    let a ← analyzeTarget map request.repositoryRoot path
    sols := sols.push a
  return (libs, sols)

private def positionToJson (p : Position) : Json :=
  match p.column with
  | some c =>
    Json.mkObj [("line", Json.num (JsonNumber.fromNat p.line)),
                ("column", Json.num (JsonNumber.fromNat c))]
  | none =>
    Json.mkObj [("line", Json.num (JsonNumber.fromNat p.line))]

private def locationToJson (l : Location) : Json :=
  let base : List (String × Json) := [("path", Json.str l.path)]
  let withStart : List (String × Json) := match l.start with
    | some s => base ++ [("start", positionToJson s)]
    | none   => base
  let full : List (String × Json) := match l.«end» with
    | some e => withStart ++ [("end", positionToJson e)]
    | none   => withStart
  Json.mkObj full

private def diagnosticToJson (d : Diagnostic) : Json :=
  let base : List (String × Json) := [
    ("severity", Json.str d.severity.toString),
    ("code", Json.str d.code),
    ("message", Json.str d.message)
  ]
  let full : List (String × Json) := match d.location with
    | some l => base ++ [("location", locationToJson l)]
    | none   => base
  Json.mkObj full

private def dependencyToJson (d : Dependency) : Json :=
  match d with
  | .internal path loc =>
    Json.mkObj [
      ("kind", Json.str "internal"),
      ("path", Json.str path),
      ("location", locationToJson loc)
    ]
  | .external name loc =>
    Json.mkObj [
      ("kind", Json.str "external"),
      ("name", Json.str name),
      ("location", locationToJson loc)
    ]
  | .unresolved key display loc =>
    Json.mkObj [
      ("kind", Json.str "unresolved"),
      ("key", Json.str key),
      ("display", Json.str display),
      ("location", locationToJson loc)
    ]

private def dependencyAnalysisJson (a : TargetAnalysis) : Json :=
  Json.mkObj [
    ("state", Json.str a.state),
    ("dependencies", Json.arr (a.dependencies.map dependencyToJson))
  ]

/-- Wire JSON for a library target (spec §6.3): includes the pending
    `symbol_analysis` block that plan 050 will populate. -/
def libraryJson (path : String) (a : TargetAnalysis) : Json :=
  Json.mkObj [
    ("path", Json.str path),
    ("dependency_analysis", dependencyAnalysisJson a),
    ("symbol_analysis", Json.mkObj [
      ("state", Json.str "partial"),
      ("symbols", Json.arr #[])
    ]),
    ("diagnostics", Json.arr (a.diagnostics.map diagnosticToJson))
  ]

/-- Wire JSON for a solution target (spec §6.3): no symbol analysis. -/
def solutionJson (id : String) (a : TargetAnalysis) : Json :=
  Json.mkObj [
    ("id", Json.str id),
    ("dependency_analysis", dependencyAnalysisJson a),
    ("diagnostics", Json.arr (a.diagnostics.map diagnosticToJson))
  ]

end Analyzer.Dependencies
