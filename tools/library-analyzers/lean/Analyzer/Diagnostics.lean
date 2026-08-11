/-!
Adapter protocol diagnostic helpers (spec §§6.8, 6.9; plan 048, plan 049).

The MVP handshake surfaces only one flavor of diagnostic: a protocol error
that stops analysis before any target-level result is produced. Plan 049
Task 2 grows `Location` with an optional source range (`start` / `end`)
that dependency edges populate while module-map errors continue to leave
those slots `none` — a `path`-only location is still valid on the wire
for diagnostics that lack a resolved source span.
-/
namespace Analyzer.Diagnostics

/-- Severity ladder shared by every language adapter. Values are lowercased
    tokens per the core protocol JSON contract. -/
inductive Severity where
  | info
  | warning
  | error
  deriving Repr, DecidableEq, Inhabited

def Severity.toString : Severity → String
  | .info    => "info"
  | .warning => "warning"
  | .error   => "error"

/-- Fatal protocol error raised when the request cannot be understood.
    The `code` is an adapter-defined stable slug; `message` is a human hint
    that omits absolute or repository-relative paths per spec §6.3. -/
structure ProtocolError where
  code    : String
  message : String
  deriving Repr

def ProtocolError.mk' (code message : String) : ProtocolError :=
  { code, message }

/-- One-based `line` with an optional Unicode-scalar-value `column`
    (spec §6.3). `column` is omitted when the source range collapses to a
    whole-line marker. -/
structure Position where
  line   : Nat
  column : Option Nat := none
  deriving Repr, Inhabited

/-- Repository-relative location for a diagnostic (spec §6.3). `path` is a
    `/`-separated repo-relative POSIX path. The optional `start` / `end`
    fields carry a source range; when `end` is populated it is exclusive.
    Module-map errors leave both spans `none` because they fire before any
    source is parsed. -/
structure Location where
  path  : String
  start : Option Position := none
  «end» : Option Position := none
  deriving Repr, Inhabited

/-- Per-target or per-request diagnostic (spec §6.3 wire shape).

    Later plans append this array into the response envelope's per-target
    `diagnostics` field. The MVP module-map path only ever emits `error`
    severities: a bijective mapping cannot proceed with any invalid entry. -/
structure Diagnostic where
  severity : Severity
  code     : String
  message  : String
  location : Option Location := none
  deriving Repr, Inhabited

end Analyzer.Diagnostics
