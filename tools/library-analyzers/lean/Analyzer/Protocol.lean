import Lean.Data.Json
import Analyzer.Diagnostics

/-!
Adapter protocol v1 bindings for the ce-lean analyzer (spec §§6.8, 6.9;
plan 048 Task 2).

Only the fields needed by the empty handshake are decoded strictly: unknown
top-level keys, wrong `schema_version`, and missing required strings are
rejected before any analysis runs. Later plans (dependency, symbol) extend
`AnalysisResponse` with per-target payloads; the response constructor here
always emits empty `libraries` / `solutions` arrays so the empty request is
mirrored one-to-one.
-/
namespace Analyzer.Protocol

open Lean

/-- Adapter protocol version implemented by this executable. -/
def schemaVersion : Nat := 1
/-- Adapter identity name reported at handshake, cross-checked against
    `language_plans::LEAN_ADAPTER_NAME` in the build driver. -/
def adapterName : String := "ce-lean"
/-- Adapter identity version reported at handshake, cross-checked against
    `language_plans::LEAN_ADAPTER_VERSION`. -/
def adapterVersion : String := "0.1.0"
/-- Toolchain name emitted alongside `Lean.versionString`. -/
def toolchainName : String := "lean"

structure ToolchainIdentity where
  name    : String
  version : String
  target  : Option String := none
  deriving Repr

structure AdapterIdentity where
  name       : String
  version    : String
  toolchains : Array ToolchainIdentity := #[]
  deriving Repr

structure LibraryTarget where
  path : String
  deriving Repr

structure SolutionTarget where
  id    : String
  root  : String
  entry : String
  deriving Repr

structure AnalysisRequest where
  schemaVersion  : Nat
  repositoryRoot : String
  language       : String
  libraries      : Array LibraryTarget := #[]
  solutions      : Array SolutionTarget := #[]
  deriving Repr

/-- Allow-list of top-level request keys. Anything outside this set fails
    the strict `denyUnknownFields` gate mirrored from the core Rust
    `AnalysisRequest`. -/
def requestKeys : List String :=
  ["schema_version", "repository_root", "language", "libraries", "solutions"]

def libraryKeys : List String := ["path"]
def solutionKeys : List String := ["id", "root", "entry"]

private def objKeys (j : Json) : Except Analyzer.Diagnostics.ProtocolError (List String) :=
  match j with
  | Json.obj m => Except.ok (m.toArray.toList.map Prod.fst)
  | _          => Except.error (.mk' "invalid_request" "top-level JSON must be an object")

private def rejectUnknown
    (raw : Json) (allowed : List String) (context : String) :
    Except Analyzer.Diagnostics.ProtocolError Unit := do
  let keys ← objKeys raw
  for k in keys do
    if !allowed.contains k then
      throw (.mk' "unknown_field" s!"unknown field {k} in {context}")

private def getStr (raw : Json) (key : String) :
    Except Analyzer.Diagnostics.ProtocolError String :=
  match raw.getObjVal? key with
  | Except.ok v =>
    match v.getStr? with
    | Except.ok s   => Except.ok s
    | Except.error _ => Except.error (.mk' "invalid_field" s!"field {key} must be a string")
  | Except.error _ => Except.error (.mk' "missing_field" s!"missing required field {key}")

private def getNat (raw : Json) (key : String) :
    Except Analyzer.Diagnostics.ProtocolError Nat :=
  match raw.getObjVal? key with
  | Except.ok v =>
    match v.getNat? with
    | Except.ok n   => Except.ok n
    | Except.error _ => Except.error (.mk' "invalid_field" s!"field {key} must be a non-negative integer")
  | Except.error _ => Except.error (.mk' "missing_field" s!"missing required field {key}")

private def getArr (raw : Json) (key : String) :
    Except Analyzer.Diagnostics.ProtocolError (Array Json) :=
  match raw.getObjVal? key with
  | Except.ok v =>
    match v.getArr? with
    | Except.ok arr  => Except.ok arr
    | Except.error _ => Except.error (.mk' "invalid_field" s!"field {key} must be an array")
  | Except.error _ => Except.error (.mk' "missing_field" s!"missing required field {key}")

private def parseLibrary (raw : Json) :
    Except Analyzer.Diagnostics.ProtocolError LibraryTarget := do
  rejectUnknown raw libraryKeys "libraries[]"
  let path ← getStr raw "path"
  return { path }

private def parseSolution (raw : Json) :
    Except Analyzer.Diagnostics.ProtocolError SolutionTarget := do
  rejectUnknown raw solutionKeys "solutions[]"
  let id ← getStr raw "id"
  let root ← getStr raw "root"
  let entry ← getStr raw "entry"
  return { id, root, entry }

/-- Parse a raw JSON document into an `AnalysisRequest`. Strict on unknown
    keys, wrong schema versions, and missing required fields, matching the
    Rust core's `#[serde(deny_unknown_fields)]` gate. -/
def parseRequest (raw : String) :
    Except Analyzer.Diagnostics.ProtocolError AnalysisRequest := do
  let j ← match Json.parse raw with
    | Except.ok j    => Except.ok j
    | Except.error e => Except.error (.mk' "invalid_json" s!"could not parse request JSON: {e}")
  rejectUnknown j requestKeys "top level"
  let schema ← getNat j "schema_version"
  if schema != schemaVersion then
    throw (.mk' "unsupported_schema_version"
      s!"unsupported schema_version {schema}; expected {schemaVersion}")
  let repositoryRoot ← getStr j "repository_root"
  let language ← getStr j "language"
  let librariesRaw ← getArr j "libraries"
  let solutionsRaw ← getArr j "solutions"
  let libraries ← librariesRaw.mapM parseLibrary
  let solutions ← solutionsRaw.mapM parseSolution
  return {
    schemaVersion  := schema
    repositoryRoot
    language
    libraries
    solutions
  }

/-- Serialize a `ToolchainIdentity` to JSON, emitting `target` only when set. -/
def toolchainToJson (t : ToolchainIdentity) : Json :=
  match t.target with
  | some tgt =>
    Json.mkObj [("name", Json.str t.name),
                ("version", Json.str t.version),
                ("target", Json.str tgt)]
  | none =>
    Json.mkObj [("name", Json.str t.name),
                ("version", Json.str t.version)]

def adapterToJson (a : AdapterIdentity) : Json :=
  let toolchains := a.toolchains.map toolchainToJson
  Json.mkObj [
    ("name", Json.str a.name),
    ("version", Json.str a.version),
    ("toolchains", Json.arr toolchains)
  ]

/-- Envelope-only response used by the empty handshake. The MVP never returns
    per-target payloads from Lean; later plans build the arrays here. -/
def emptyResponseJson (adapter : AdapterIdentity) : Json :=
  Json.mkObj [
    ("schema_version", Json.num (JsonNumber.fromNat schemaVersion)),
    ("adapter", adapterToJson adapter),
    ("libraries", Json.arr #[]),
    ("solutions", Json.arr #[])
  ]

/-- Build the handshake adapter identity. `leanVersion` is threaded in from
    `Main` so callers can pass either the compile-time `Lean.versionString`
    or a test double. -/
def handshakeAdapter (leanVersion : String) : AdapterIdentity :=
  { name       := adapterName
    version    := adapterVersion
    toolchains := #[{ name := toolchainName, version := leanVersion }]
  }

end Analyzer.Protocol
