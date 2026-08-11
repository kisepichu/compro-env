import Lean.Data.Json
import Analyzer.Protocol
import Analyzer.Diagnostics
import Analyzer.ModuleMap
import Analyzer.Dependencies

/-!
`ce-lean` — Lean language analyzer entry point for compro-env
(spec §§6.8, 6.9; plan 048 Task 2 + plan 049 Task 2).

Reads a single `AnalysisRequest` JSON document from stdin, validates the
protocol shape, builds a bijective `ModuleMap` from the request's managed
`.lean` paths, and writes a single `AnalysisResponse` JSON document to
stdout. When the request lists no targets, the general analysis path
naturally emits empty `libraries` / `solutions` arrays — the same
envelope the empty handshake asserts against.
-/

open Lean

def main : IO UInt32 := do
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
      let libsJson : Array Json := (req.libraries.zip libAnalyses).map
        (fun ⟨lib, a⟩ => Analyzer.Dependencies.libraryJson lib.path a)
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
