import Lean
import Analyzer.Protocol
import Analyzer.Diagnostics
import Analyzer.Elaboration

/-!
Symbol projection for the ce-lean adapter (spec §6.8; plan 050 Task 2).

Diffs each `ElaboratedTarget`'s environment against its imports-only
baseline, keeps declarations whose `declRange` was recorded during
elaboration (Lean marks kernel-synthesized recursors, `noConfusion`
helpers, `sizeOf` specs, and other generated helpers with no range), and
projects Lean's `ConstantInfo` into the adapter's stable kind vocabulary:

* `def`, `abbrev`, `opaque`
* `theorem`, `axiom`
* `inductive`, `structure`, `class`
* `constructor`, `field`
* `instance`

Constructor / field distinction: for structures — including classes,
which the kernel represents as one-constructor inductives — every
`getStructureFields` name projects out as `defnInfo` with a source
range; we relabel those from `def` to `field`. Instances (either the
`instance` command or `@[instance] def`) are detected via
`Lean.Meta.isInstanceCore`.

`search_names` always contain `name`, plus `qualified_name` when the
declaration lives under a namespace; the core deduplicates. Symbols are
sorted by (line, column, qualified name / name) so downstream fixtures
compare byte-for-byte across repeated runs.
-/

namespace Analyzer.Symbols

open Lean
open Analyzer.Protocol
open Analyzer.Diagnostics
open Analyzer.Elaboration

/-- One projected declaration. Wire-shape mirrors the shared library
    adapter protocol `Symbol` (spec §6.3). -/
structure Symbol where
  name          : String
  qualifiedName : Option String := none
  searchNames   : Array String
  kind          : String
  location      : Option Location := none
  deriving Repr, Inhabited

/-- Everything an adapter emits per target: the state (`complete` /
    `partial` / `failed`) plus the ordered symbol array. -/
structure SymbolAnalysis where
  state   : String
  symbols : Array Symbol
  deriving Repr, Inhabited

/-- Names starting with these substrings are elaborator-internal
    helpers we never want to emit even if they carry a range. -/
private def isInternalNamePart (part : String) : Bool :=
  part.startsWith "_aux_"      ||
  part.startsWith "_@"         ||
  part.startsWith "_hyg"       ||
  part.startsWith "_hygFn"     ||
  part.startsWith "_regBuiltin"

/-- Recursively check every dot-separated component of a `Name` for an
    elaborator-internal prefix. -/
private partial def nameIsInternal : Name → Bool
  | .anonymous       => false
  | .str p s         => isInternalNamePart s || nameIsInternal p
  | .num p _         => nameIsInternal p

/-- Position from Lean's 1-based line / 0-based column to the adapter's
    1-based Unicode-scalar column. -/
private def positionOf (p : Lean.Position) : Analyzer.Diagnostics.Position :=
  { line := p.line, column := some (p.column + 1) }

private def locationOfRange
    (repoRelPath : String) (r : DeclarationRanges) : Location :=
  { path := repoRelPath
    start := some (positionOf r.range.pos)
    «end» := some (positionOf r.range.endPos) }

/-- Convert a `Lean.Name` to a search-friendly dotted string. Escaped
    names (`«...»`) are unwrapped by `toString` already. -/
private def nameToString (n : Name) : String := n.toString

/-- Split a `Lean.Name` into (qualifier, last component). Anonymous
    names — none should reach here — collapse to a blank pair. -/
private def splitTail : Name → String × Option String
  | .str p s =>
    if p.isAnonymous then (s, none)
    else (s, some (nameToString (.str p s)))
  | .num p n =>
    -- Numeric tail: keep the whole name as its own display.
    let whole := nameToString (.num p n)
    (whole, none)
  | .anonymous => ("", none)

/-- Adapter kind vocabulary (spec §6.3 requires `[a-z][a-z0-9_-]*`). -/
private def projectKind
    (env : Environment) (name : Name) (ci : ConstantInfo) : Option String :=
  match ci with
  | .thmInfo   _ => some "theorem"
  | .axiomInfo _ => some "axiom"
  | .opaqueInfo _ => some "opaque"
  | .ctorInfo  _ => some "constructor"
  | .inductInfo _ =>
    if isClass env name then some "class"
    else if isStructure env name then some "structure"
    else some "inductive"
  | .defnInfo di =>
    if Lean.Meta.isInstanceCore env name then some "instance"
    else
      -- Structure / class fields are elaborated as `defnInfo` at the
      -- field's source range with `.abbrev` hints (both projections and
      -- class methods). Check the parent's field list first so we do
      -- not label a real field as a top-level abbrev.
      let parentField : Bool :=
        match name with
        | .str parent suffix =>
          if isStructure env parent then
            (getStructureFields env parent).contains (Name.mkSimple suffix)
          else false
        | _ => false
      if parentField then some "field"
      else
        let isAbbrev : Bool := match di.hints with
          | .abbrev => true
          | _       => false
        if isAbbrev then some "abbrev"
        else some "def"
  | .recInfo _ | .quotInfo _ => none

private def symbolOfConstant
    (repoRelPath : String)
    (env : Environment)
    (name : Name)
    (ci : ConstantInfo) : Option Symbol := do
  if nameIsInternal name then none
  else
    let ranges ← declRangeExt.find? env name
    let kind ← projectKind env name ci
    let (display, qualified) := splitTail name
    let searchNames : Array String := match qualified with
      | some q => #[display, q]
      | none   => #[display]
    return {
      name := display
      qualifiedName := qualified
      searchNames
      kind
      location := some (locationOfRange repoRelPath ranges)
    }

/-- Collect the constant names added by this target's body — the diff
    between the final environment and the imports-only baseline captured
    inside `elaborateSource`. -/
private def newConstantNames (target : ElaboratedTarget) : Array Name :=
  target.environment.constants.foldStage2 (s := (#[] : Array Name))
    fun acc n _ => if target.baseline.contains n then acc else acc.push n

/-- Determine adapter symbol state.
    * `failed` when both `hasErrors` and no symbols survived.
    * `partial` when either `hasErrors` or a projection was dropped.
    * `complete` otherwise. -/
private def symbolState
    (hasErrors : Bool) (symbolCount : Nat) : String :=
  if hasErrors && symbolCount == 0 then "failed"
  else if hasErrors then "partial"
  else "complete"

/-- Sort symbols by source location, then qualified/display name.
    Deterministic across runs so protocol fixtures stay stable. -/
private def sortSymbols (syms : Array Symbol) : Array Symbol :=
  syms.qsort fun a b =>
    let posOf : Symbol → Nat × Nat := fun s =>
      match s.location.bind (·.start) with
      | some p => (p.line, p.column.getD 0)
      | none   => (0, 0)
    let (la, ca) := posOf a
    let (lb, cb) := posOf b
    if la != lb then la < lb
    else if ca != cb then ca < cb
    else
      let ka := a.qualifiedName.getD a.name
      let kb := b.qualifiedName.getD b.name
      ka < kb

/-- Extract every symbol authored by `target`. -/
def extractSymbols (target : ElaboratedTarget) : SymbolAnalysis := Id.run do
  let env := target.environment
  let newNames := newConstantNames target
  let mut symbols : Array Symbol := #[]
  for n in newNames do
    match env.constants.find? n with
    | some ci =>
      match symbolOfConstant target.path env n ci with
      | some sym => symbols := symbols.push sym
      | none     => pure ()
    | none => pure ()
  let sorted := sortSymbols symbols
  return { state := symbolState target.hasErrors sorted.size, symbols := sorted }

private def positionToJson (p : Analyzer.Diagnostics.Position) : Json :=
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

def symbolToJson (s : Symbol) : Json :=
  let base : List (String × Json) := [
    ("name", Json.str s.name)
  ]
  let withQualified : List (String × Json) := match s.qualifiedName with
    | some q => base ++ [("qualified_name", Json.str q)]
    | none   => base
  let withSearchNames : List (String × Json) :=
    withQualified ++ [("search_names",
      Json.arr (s.searchNames.map Json.str))]
  let withKind : List (String × Json) :=
    withSearchNames ++ [("kind", Json.str s.kind)]
  let full : List (String × Json) := match s.location with
    | some l => withKind ++ [("location", locationToJson l)]
    | none   => withKind
  Json.mkObj full

def symbolAnalysisToJson (a : SymbolAnalysis) : Json :=
  Json.mkObj [
    ("state", Json.str a.state),
    ("symbols", Json.arr (a.symbols.map symbolToJson))
  ]

end Analyzer.Symbols
