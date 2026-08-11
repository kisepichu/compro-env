import Lean.Data.Json
import Analyzer.Protocol
import Analyzer.Diagnostics

/-!
`ce-lean` — Lean language analyzer entry point for compro-env
(spec §§6.8, 6.9; plan 048 Task 2).

Reads a single `AnalysisRequest` JSON document from stdin, validates the
protocol shape, and writes a single `AnalysisResponse` JSON document to
stdout. The MVP handshake path returns adapter identity + observed Lean
toolchain identity only; dependency and symbol extraction are added by
follow-up plans and reuse the shared parser here.
-/

open Lean

def main : IO UInt32 := do
  let stdin ← IO.getStdin
  let raw ← stdin.readToEnd
  match Analyzer.Protocol.parseRequest raw with
  | .error e =>
    IO.eprintln s!"ce-lean: error: [{e.code}] {e.message}"
    return 1
  | .ok _req =>
    let adapter := Analyzer.Protocol.handshakeAdapter Lean.versionString
    let resp := Analyzer.Protocol.emptyResponseJson adapter
    IO.println resp.compress
    return 0
