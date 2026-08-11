import Lean.Data.Json
import Analyzer.Protocol
import Analyzer.Diagnostics

/-!
Handshake test for the Lean adapter (spec §§6.8, 6.9; plan 048 Task 2).

Runs the parser + response builder against the shared fixtures in
`tools/library-analyzers/protocol/fixtures/`. The empty-request fixture
must yield the exact identity envelope the build driver's handshake gate
expects: adapter `ce-lean` @ `0.1.0`, one `lean` toolchain, and no
libraries or solutions.

The Lean release string is threaded in explicitly to lock the response
envelope shape, and `Lean.versionString` — the value `Analyzer.Main`
actually reports at runtime — is asserted against the same expected pin
so a toolchain drift at build time surfaces here as a mismatch instead
of silently changing the reported identity.
-/

open Lean

namespace Analyzer.Tests

private def expectedLeanVersion : String := "4.30.0"

private def fixtureDir : IO System.FilePath := do
  match ← IO.getEnv "CE_LEAN_FIXTURE_DIR" with
  | some p => return System.FilePath.mk p
  | none   => return System.FilePath.mk "../protocol/fixtures"

private def readFixture (name : String) : IO String := do
  let dir ← fixtureDir
  IO.FS.readFile (dir / name)

private def fail (msg : String) : IO Unit := do
  IO.eprintln s!"handshake test failure: {msg}"
  discard <| (IO.Process.exit 1 : IO UInt32)

private def check (cond : Bool) (msg : String) : IO Unit :=
  if cond then pure () else fail msg

/-- `Lean.versionString` — the value `Analyzer.Main` reports at runtime —
    matches the pinned Lean 4.30.0 release. This trips when the checked-in
    `lean-toolchain` is bumped without updating the expected constant. -/
def testLeanVersionStringMatchesPin : IO Unit := do
  check (Lean.versionString == expectedLeanVersion)
    s!"Lean.versionString = {Lean.versionString}, expected {expectedLeanVersion}"

/-- The response constructor's identity envelope matches the expected
    handshake shape when the request is an empty analysis. -/
def testEmptyResponseShape : IO Unit := do
  let adapter := Analyzer.Protocol.handshakeAdapter expectedLeanVersion
  check (adapter.name == "ce-lean") s!"adapter.name = {adapter.name}"
  check (adapter.version == "0.1.0") s!"adapter.version = {adapter.version}"
  check (adapter.toolchains.size == 1) s!"toolchains.size = {adapter.toolchains.size}"
  let t := adapter.toolchains[0]!
  check (t.name == "lean") s!"toolchain.name = {t.name}"
  check (t.version == expectedLeanVersion) s!"toolchain.version = {t.version}"
  let resp := Analyzer.Protocol.emptyResponseJson adapter
  match resp.getObjVal? "schema_version" with
  | .ok v =>
    match v.getNat? with
    | .ok n   => check (n == 1) s!"schema_version = {n}"
    | .error _ => (fail "schema_version was not a number")
  | .error _ => (fail "schema_version missing")
  match resp.getObjVal? "libraries" with
  | .ok v =>
    match v.getArr? with
    | .ok a   => check (a.isEmpty) s!"libraries not empty ({a.size})"
    | .error _ => (fail "libraries not an array")
  | .error _ => (fail "libraries missing")
  match resp.getObjVal? "solutions" with
  | .ok v =>
    match v.getArr? with
    | .ok a   => check (a.isEmpty) s!"solutions not empty ({a.size})"
    | .error _ => (fail "solutions not an array")
  | .error _ => (fail "solutions missing")

/-- The strict parser accepts the shared empty-request fixture and rejects
    both unknown top-level keys and mismatched schema versions. -/
def testParseFixtures : IO Unit := do
  let raw ← readFixture "empty-request.json"
  match Analyzer.Protocol.parseRequest raw with
  | .error e => (fail s!"empty fixture rejected: [{e.code}] {e.message}")
  | .ok req  =>
    check (req.schemaVersion == 1) s!"fixture schema_version = {req.schemaVersion}"
    check (req.libraries.isEmpty) "fixture libraries not empty"
    check (req.solutions.isEmpty) "fixture solutions not empty"
  let bad := "{\"schema_version\": 2, \"repository_root\": \".\", \"language\": \"lean\", \"libraries\": [], \"solutions\": []}"
  match Analyzer.Protocol.parseRequest bad with
  | .error _ => pure ()
  | .ok _    => (fail "schema_version = 2 should be rejected")
  let unknown := "{\"schema_version\": 1, \"repository_root\": \".\", \"language\": \"lean\", \"libraries\": [], \"solutions\": [], \"extra\": 1}"
  match Analyzer.Protocol.parseRequest unknown with
  | .error _ => pure ()
  | .ok _    => (fail "unknown top-level key should be rejected")

end Analyzer.Tests

def main : IO Unit := do
  Analyzer.Tests.testLeanVersionStringMatchesPin
  Analyzer.Tests.testEmptyResponseShape
  Analyzer.Tests.testParseFixtures
  IO.println "ce-lean handshake tests passed"
