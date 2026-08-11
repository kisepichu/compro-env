import Lean.Data.Json
import Analyzer.Protocol
import Analyzer.Diagnostics
import Analyzer.ModuleMap
import Analyzer.Dependencies
import Analyzer.Elaboration
import Analyzer.Symbols

/-!
`ce-lean` — Lean language analyzer entry point for compro-env
(spec §§6.8, 6.9; plans 048 / 049 / 050).

Reads a single `AnalysisRequest` JSON document from stdin, validates the
protocol shape, builds a bijective `ModuleMap` from the request's managed
`.lean` paths, runs the direct-dependency analyzer (plan 049) and the
elaborator + symbol projector (plan 050) in topological order, and writes
a single `AnalysisResponse` JSON document to stdout. When the request
lists no targets, the general analysis path naturally emits empty
`libraries` / `solutions` arrays — the same envelope the empty handshake
asserts against.

`unsafe def unsafeMain` arms Lean's initializer table before the first
`importModules` call so downstream elaboration can enter the sysroot
search path safely; the safe `main` opaquely dispatches to it via
`@[implemented_by]`.
-/

open Lean

unsafe def unsafeMain : IO UInt32 := do
  let stdin ← IO.getStdin
  let raw ← stdin.readToEnd
  match Analyzer.Protocol.parseRequest raw with
  | .error e =>
    IO.eprintln s!"ce-lean: error: [{e.code}] {e.message}"
    return 1
  | .ok req =>
    match Analyzer.ModuleMap.buildModuleMap req with
    | .error d =>
      IO.eprintln s!"ce-lean: error: [{d.code}] {d.message}"
      return 1
    | .ok map =>
      let (libAnalyses, solAnalyses) ← Analyzer.Dependencies.analyzeRequest map req
      let libPairs : Array (Analyzer.ModuleMap.Entry × Analyzer.Dependencies.TargetAnalysis) :=
        (req.libraries.zip libAnalyses).map fun ⟨lib, a⟩ =>
          -- Look up the matching module-map entry by path so the internal-
          -- dep projector can key off it. `find?` cannot miss: every library
          -- path contributed exactly one entry to the bijective map.
          match map.entries.find? (fun e => e.path == lib.path) with
          | some e => (e, a)
          | none   =>
            (({ path := lib.path, moduleName := .anonymous }
              : Analyzer.ModuleMap.Entry), a)
      let depsMap := Analyzer.Elaboration.internalDepsMap map libPairs
      let elaborated ← Analyzer.Elaboration.elaborateTargets req map depsMap
      let libsJson : Array Json := (req.libraries.zip libAnalyses).map fun ⟨lib, a⟩ =>
        -- Match the elaborated result back to this library by path. When
        -- the elaborator reported nothing (e.g. Task 3 rejects the whole
        -- batch upstream), fall back to a `partial` empty analysis so the
        -- envelope stays well-formed.
        let matched := elaborated.find? (·.path == lib.path)
        let sym : Analyzer.Symbols.SymbolAnalysis := match matched with
          | some t => Analyzer.Symbols.extractSymbols t
          | none   => { state := "partial", symbols := #[] }
        -- Fold the elaborator's error/warning diagnostics into the
        -- dependency analysis's own diagnostics so callers see every
        -- source of trouble in one array.
        let elabDiags : Array Analyzer.Diagnostics.Diagnostic :=
          match matched with
          | some t => t.diagnostics
          | none   => #[]
        let a' := { a with diagnostics := a.diagnostics ++ elabDiags }
        Analyzer.Dependencies.libraryJsonWithSymbols lib.path a'
          (Analyzer.Symbols.symbolAnalysisToJson sym)
      let solsJson : Array Json := (req.solutions.zip solAnalyses).map
        (fun ⟨sol, a⟩ => Analyzer.Dependencies.solutionJson sol.id a)
      let adapter := Analyzer.Protocol.handshakeAdapter Lean.versionString
      let resp := Json.mkObj [
        ("schema_version",
          Json.num (JsonNumber.fromNat Analyzer.Protocol.schemaVersion)),
        ("adapter", Analyzer.Protocol.adapterToJson adapter),
        ("libraries", Json.arr libsJson),
        ("solutions", Json.arr solsJson)
      ]
      IO.println resp.compress
      return 0

@[implemented_by unsafeMain]
opaque mainOpaque : IO UInt32

def main : IO UInt32 := mainOpaque
