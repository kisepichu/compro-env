import Lean
import Analyzer.Protocol
import Analyzer.Diagnostics
import Analyzer.ModuleMap
import Analyzer.Dependencies

/-!
Deterministic Lean module elaboration for the ce-lean adapter
(spec §§6.8, 6.9; plan 050 Task 1).

Each managed target is elaborated once with `Lean.Elab.processHeader` +
`Lean.Elab.IO.processCommands` under a fresh `Environment`. Targets are
processed in topological order over the internal-dep graph produced by
`Analyzer.Dependencies`; cycles fall through in the surrounding
path-sorted order so output stays stable across runs. The prepared Lean
toolchain provides `.olean`s on `LEAN_PATH` for external imports (`Init`,
`Std`, `Lean`); managed-internal imports are not built to disk in this
adapter, so a target that transitively imports another managed target
generally reports an elaboration error and its symbol analysis degrades
to `partial` — Task 2 attaches that state to the wire response.
-/

namespace Analyzer.Elaboration

open Lean
open Analyzer.Protocol
open Analyzer.Diagnostics
open Analyzer.ModuleMap
open Analyzer.Dependencies

/-- One target's elaboration result. `baseline` is the constant-name set
    right after `processHeader` (imports only), so the diff against
    `environment.constants` yields the declarations authored by this
    target. `hasErrors` is `true` when either header or body produced an
    error-severity Lean message; `diagnostics` carries the adapter-shaped
    error/warning envelopes for the wire response. -/
structure ElaboratedTarget where
  path        : String
  moduleName  : Name
  source      : String
  baseline    : NameSet
  environment : Environment
  hasErrors   : Bool
  diagnostics : Array Diagnostic

/-- Fixed-point topological ordering. Emit every entry whose internal
    deps have already been emitted, iterate until progress stalls, then
    flush the remainder in the surrounding path-sorted order so cycles
    do not deadlock and the output stays stable. -/
def topologicalOrder
    (map : ModuleMap) (depsMap : NameMap (Array Name)) : Array Entry := Id.run do
  let mut result : Array Entry := #[]
  let mut emitted : NameSet := {}
  let mut remaining : Array Entry := map.entries
  while remaining.size > 0 do
    let mut progressed : Bool := false
    let mut nextRemaining : Array Entry := #[]
    for e in remaining do
      let deps := (depsMap.find? e.moduleName).getD #[]
      let allEmitted := deps.all (fun d => emitted.contains d)
      if allEmitted then
        result := result.push e
        emitted := emitted.insert e.moduleName
        progressed := true
      else
        nextRemaining := nextRemaining.push e
    if progressed then
      remaining := nextRemaining
    else
      for e in nextRemaining do
        result := result.push e
        emitted := emitted.insert e.moduleName
      remaining := #[]
  return result

/-- Project the `Dependencies.analyzeTarget` output into an internal-only
    dep map keyed by module name. External and unresolved edges do not
    participate in the topological order. -/
def internalDepsMap
    (map : ModuleMap)
    (analyses : Array (Entry × TargetAnalysis)) : NameMap (Array Name) :=
  analyses.foldl (init := (∅ : NameMap (Array Name))) fun acc pair =>
    let e := pair.fst
    let a := pair.snd
    let deps : Array Name := a.dependencies.filterMap fun
      | .internal path _ =>
        match map.entries.find? (fun x => x.path == path) with
        | some x => some x.moduleName
        | none   => none
      | _ => none
    acc.insert e.moduleName deps

private def leanPositionOf (p : Lean.Position) :
    Analyzer.Diagnostics.Position :=
  { line := p.line, column := some (p.column + 1) }

private def messageLocation
    (repoRelPath : String) (m : Lean.Message) : Location :=
  { path  := repoRelPath
    start := some (leanPositionOf m.pos)
    «end» := m.endPos.map leanPositionOf }

private def firstErrorLocation
    (repoRelPath : String)
    (headerMsgs : MessageLog)
    (bodyMsgs   : MessageLog) : Option Location := Id.run do
  for m in headerMsgs.toList ++ bodyMsgs.toList do
    if m.severity == .error then
      return some (messageLocation repoRelPath m)
  return none

/-- Fold elaboration errors and warnings into at most two stable
    diagnostics per target. The concrete Lean error text is emitted to
    stderr where operators can read it in full; we keep the wire
    response fixture-stable so downstream compare-against-checked-in-JSON
    tests do not break on unrelated Lean chatter tweaks. -/
private def collectDiagnostics
    (repoRelPath : String)
    (headerMsgs : MessageLog)
    (bodyMsgs   : MessageLog) : IO (Array Diagnostic) := do
  -- Emit every raw Lean message to stderr so callers still see the
  -- elaborator output; spec §6.3 lets adapters stream anything they
  -- like there.
  for m in headerMsgs.toList ++ bodyMsgs.toList do
    match m.severity with
    | .error | .warning =>
      IO.eprintln s!"ce-lean [{repoRelPath}] {(← m.data.toString).trimAscii}"
    | _ => pure ()
  let hasError := headerMsgs.hasErrors || bodyMsgs.hasErrors
  let mut hasWarning := false
  for m in headerMsgs.toList ++ bodyMsgs.toList do
    if m.severity == .warning then hasWarning := true
  let mut out : Array Diagnostic := #[]
  if hasError then
    out := out.push {
      severity := .error
      code := "lean.symbols.elaboration"
      message := "target elaboration produced errors; symbols may be incomplete"
      location := firstErrorLocation repoRelPath headerMsgs bodyMsgs
    }
  if hasWarning then
    out := out.push {
      severity := .warning
      code := "lean.symbols.elaboration"
      message := "target elaboration produced warnings"
      location := some { path := repoRelPath }
    }
  return out

/-- Elaborate a target's source string. Returns the final environment
    plus the constant-name baseline captured immediately after import
    processing (so callers can diff the target's own declarations). -/
unsafe def elaborateSourceUnsafe
    (repoRelPath : String)
    (source      : String)
    (moduleName  : Name) : IO ElaboratedTarget := do
  -- Initializers must be armed before every `importModules` call — a
  -- single arming at process start is not enough for follow-up imports.
  Lean.enableInitializersExecution
  let inputCtx := Parser.mkInputContext source repoRelPath
  let (header, parserState, headerMsgs) ← Parser.parseHeader inputCtx
  let (env, msgsAfterHeader) ←
    Lean.Elab.processHeader (mainModule := moduleName) header {} headerMsgs inputCtx
  let baseline : NameSet :=
    env.constants.foldStage2 (s := NameSet.empty) (fun s n _ => s.insert n)
  let commandState := Lean.Elab.Command.mkState env msgsAfterHeader
  let s ← Lean.Elab.IO.processCommands inputCtx parserState commandState
  let bodyMsgs := s.commandState.messages
  let hasErrors := msgsAfterHeader.hasErrors || bodyMsgs.hasErrors
  let diagnostics ← collectDiagnostics repoRelPath msgsAfterHeader bodyMsgs
  return {
    path := repoRelPath
    moduleName
    source
    baseline
    environment := s.commandState.env
    hasErrors
    diagnostics
  }

/-- Read the target file and elaborate it. Missing files yield an
    `elaboration.read` error diagnostic and an empty environment so the
    caller can still report a `failed` symbol analysis. -/
unsafe def elaborateTargetUnsafe
    (repositoryRoot : String) (entry : Entry) : IO ElaboratedTarget := do
  let absPath : System.FilePath :=
    System.FilePath.mk repositoryRoot / entry.path
  let sourceExists ← absPath.pathExists
  if !sourceExists then
    return {
      path := entry.path
      moduleName := entry.moduleName
      source := ""
      baseline := {}
      environment := ← mkEmptyEnvironment
      hasErrors := true
      diagnostics := #[{
        severity := .error
        code := "lean.symbols.read"
        message := s!"source file not found: {entry.path}"
        location := some { path := entry.path }
      }]
    }
  let source ← IO.FS.readFile absPath
  elaborateSourceUnsafe entry.path source entry.moduleName

/-- Elaborate every managed target in stable topological order.

    * `request`  — the analysis request; its `repositoryRoot` roots on-disk
      reads.
    * `moduleMap` — bijective path↔module map from `ModuleMap`.
    * `depsMap`   — internal-only dep graph from `internalDepsMap`.

    The returned array preserves the topological order so callers can
    correlate the response envelope's `libraries`/`solutions` arrays back
    to it in request order by module name. -/
unsafe def elaborateTargetsUnsafe
    (request : AnalysisRequest)
    (moduleMap : ModuleMap)
    (depsMap : NameMap (Array Name)) : IO (Array ElaboratedTarget) := do
  Lean.initSearchPath (← Lean.findSysroot)
  let ordered := topologicalOrder moduleMap depsMap
  let mut out : Array ElaboratedTarget := #[]
  for entry in ordered do
    out := out.push (← elaborateTargetUnsafe request.repositoryRoot entry)
  return out

@[implemented_by elaborateTargetsUnsafe]
opaque elaborateTargets
    (request : AnalysisRequest)
    (moduleMap : ModuleMap)
    (depsMap : NameMap (Array Name)) : IO (Array ElaboratedTarget)

@[implemented_by elaborateSourceUnsafe]
opaque elaborateSource
    (repoRelPath : String) (source : String) (moduleName : Name) :
    IO ElaboratedTarget

end Analyzer.Elaboration
