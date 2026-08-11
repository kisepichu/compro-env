import Lean.Data.Name
import Analyzer.Protocol
import Analyzer.Diagnostics

/-!
Lean adapter module map (spec §6.8; plan 049 Task 1).

Maps every managed `.lean` source path from an `AnalysisRequest` to its Lean
module name and back. Every `LibraryTarget.path` and every
`SolutionTarget.<root>/<entry>` contributes one entry. The mapping is
required to be bijective — paths that would resolve to the same module name
are rejected before any source is parsed.

Module names are derived from a repo-relative path with the following
convention (spec §6.8 does not mandate an exact prefix, so we use the
simplest bijection-compatible rule):

1. The path must end in `.lean`; the extension is stripped.
2. If any path segment literally equals `lean` and there is at least one
   segment after it, everything up to and including the LAST such segment
   is dropped. This lets `libraries/lean/Foo/Bar.lean` become `Foo.Bar`
   without hard-coding `libraries/lean` as the sole managed root, and lets
   a solution that keeps `.lean` sources under `.../lean/pkg/` be stripped
   the same way. When no such segment exists, the full repo-relative stem
   is used, so misconfigured roots surface as long unambiguous identifiers
   rather than silent collisions.
3. The remaining `/`-separated components are joined into a Lean `Name`
   with `Name.mkStr` (`Foo` + `Bar` → `Foo.Bar`).

Every surviving component is validated as a Lean identifier component:
non-empty, does not start with an ASCII digit, and contains no `.`. Path
validation additionally rejects absolute paths, `..` segments, and any
path that does not end in `.lean`.

Diagnostics use adapter-defined slug codes: `lean.module_map.not_lean`,
`lean.module_map.repository_escape`, `lean.module_map.invalid_component`,
and `lean.module_map.duplicate_owner`. All are `error` severity — a
bijective mapping cannot proceed with any invalid entry.

Entries are stored in `path` UTF-8 byte order for stable iteration. UTF-8
byte comparison equals Unicode codepoint comparison for well-formed UTF-8,
which is what Lean's default `String.lt` implements.
-/

namespace Analyzer.ModuleMap

open Lean
open Analyzer.Diagnostics
open Analyzer.Protocol

/-- One resolved path/module pair. `path` is repository-relative POSIX. -/
structure Entry where
  path       : String
  moduleName : Name
  deriving Repr, Inhabited

/-- Bijective adapter between managed `.lean` paths and Lean module names.
    Entries are sorted by `path` in UTF-8 byte (== Unicode codepoint) order
    so downstream consumers observe a stable iteration order. -/
structure ModuleMap where
  entries : Array Entry := #[]
  deriving Repr, Inhabited

/-- Build an `error` diagnostic. The offending path, when known, is threaded
    through the optional `Location` so operators can spot the culprit without
    Lean absolute paths leaking (spec §6.3 forbids absolute paths in the
    location shape). -/
private def err (code msg : String) (path : Option String := none) :
    Diagnostic :=
  { severity := .error
    code
    message  := msg
    location := path.map (fun p => ({ path := p } : Location)) }

private def notLean (path : String) : Diagnostic :=
  err "lean.module_map.not_lean"
    s!"path does not end in .lean: {path}" (some path)

private def repositoryEscape (path reason : String) : Diagnostic :=
  -- No `location` — spec §6.3 requires `location.path` to be a safe
  -- repo-relative path, which is precisely what this diagnostic flags as
  -- absent. The offending path is retained in `message` for debug output.
  err "lean.module_map.repository_escape"
    s!"path {reason}: {path}" none

private def invalidComponent (path component reason : String) : Diagnostic :=
  err "lean.module_map.invalid_component"
    s!"component {reason}: {component} (in {path})" (some path)

private def duplicateOwner (msg path : String) : Diagnostic :=
  err "lean.module_map.duplicate_owner" msg (some path)

/-- Reject absolute paths and any `..` segment. Purely lexical — no file
    system access is performed. -/
private def rejectEscape (path : String) : Except Diagnostic Unit := do
  if path.startsWith "/" then
    throw (repositoryEscape path "is absolute")
  for seg in path.splitOn "/" do
    if seg == ".." then
      throw (repositoryEscape path "contains ..")

/-- Require and strip the trailing `.lean` extension. -/
private def stripLeanSuffix (path : String) : Except Diagnostic String :=
  let suffix := ".lean"
  if path.endsWith suffix then
    .ok (path.dropRight suffix.length)
  else
    .error (notLean path)

/-- Drop everything up to and including the last path segment literally
    equal to `lean`, unless that segment is the final one (files literally
    named `lean.lean` at a repo root keep the `lean` segment so the module
    name is not empty). Returns the input unchanged when no such segment
    exists. -/
private def dropUpToLastLeanSegment (segments : List String) : List String :=
  match segments.reverse with
  | [] => []
  | last :: rest =>
    let before := rest.takeWhile (fun s => s != "lean")
    if before.length == rest.length then
      -- `lean` not present in any position other than possibly `last`.
      segments
    else
      (last :: before).reverse

/-- Validate one module-name component: non-empty, no leading ASCII digit,
    no embedded `.`. -/
private def validateComponent (path component : String) :
    Except Diagnostic Unit := do
  if component.isEmpty then
    throw (invalidComponent path component "is empty")
  if component.contains '.' then
    throw (invalidComponent path component "contains '.'")
  match component.data.head? with
  | some c =>
    if c.isDigit then
      throw (invalidComponent path component "starts with a digit")
  | none => throw (invalidComponent path component "is empty")

/-- Derive a Lean `Name` from a repo-relative path.

    Fails with a `Diagnostic` when the path escapes the repository, is not a
    `.lean` file, or yields an empty / invalid module name after the
    language-root prefix is stripped. -/
def moduleNameOf (path : String) : Except Diagnostic Name := do
  rejectEscape path
  let stem ← stripLeanSuffix path
  let components := stem.splitOn "/"
  let modComponents := dropUpToLastLeanSegment components
  if modComponents.isEmpty then
    throw (invalidComponent path ""
      "yields empty module name after language-root stripping")
  let mut name : Name := .anonymous
  for c in modComponents do
    validateComponent path c
    name := name.mkStr c
  return name

/-- Concatenate a solution's `root` and `entry` into a single repo-relative
    path. Solution roots are usually non-empty and slash-free at the tail,
    but we tolerate a trailing `/` defensively. -/
private def joinRootEntry (root entry : String) : String :=
  if root.isEmpty then entry
  else if root.endsWith "/" then root ++ entry
  else s!"{root}/{entry}"

/-- Collect (path, moduleName) pairs for every target in the request. Fails
    on the first invalid path; subsequent paths are not inspected. -/
private def collectEntries (request : AnalysisRequest) :
    Except Diagnostic (Array Entry) := do
  let mut entries : Array Entry := #[]
  for lib in request.libraries do
    let name ← moduleNameOf lib.path
    entries := entries.push { path := lib.path, moduleName := name }
  for sol in request.solutions do
    let path := joinRootEntry sol.root sol.entry
    let name ← moduleNameOf path
    entries := entries.push { path, moduleName := name }
  return entries

/-- Order two entries by their path in UTF-8 byte order (== Unicode
    codepoint order for well-formed UTF-8). -/
private def entryLt (a b : Entry) : Bool := a.path < b.path

/-- Enforce that no two entries share a path (adjacent after path-sort) or a
    module name (O(n²) scan; n is small in practice). -/
private def enforceBijection (sorted : Array Entry) :
    Except Diagnostic Unit := do
  for i in [1:sorted.size] do
    let prev := sorted[i-1]!
    let cur := sorted[i]!
    if prev.path == cur.path then
      throw (duplicateOwner
        s!"path appears in the request more than once: {cur.path}"
        cur.path)
  for i in [0:sorted.size] do
    for j in [i+1:sorted.size] do
      let a := sorted[i]!
      let b := sorted[j]!
      if a.moduleName == b.moduleName then
        throw (duplicateOwner
          s!"module {a.moduleName} claimed by both {a.path} and {b.path}"
          b.path)

/-- Build a bijective `ModuleMap` from an `AnalysisRequest`.

    Fails with a single `Diagnostic` when any path is malformed, any module
    component fails identifier validation, or two paths would resolve to
    the same module name. Entries are returned sorted by `path` UTF-8 byte
    order. -/
def buildModuleMap (request : AnalysisRequest) :
    Except Diagnostic ModuleMap := do
  let raw ← collectEntries request
  let sorted := raw.qsort entryLt
  enforceBijection sorted
  return { entries := sorted }

/-- Look up the module name owning a repo-relative `path`. Returns `none`
    when the path is not managed by this analysis. -/
def moduleForPath (map : ModuleMap) (path : String) : Option Name :=
  match map.entries.find? (fun e => e.path == path) with
  | some e => some e.moduleName
  | none   => none

end Analyzer.ModuleMap
