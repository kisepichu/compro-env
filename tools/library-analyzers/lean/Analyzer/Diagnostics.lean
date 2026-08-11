/-!
Adapter protocol diagnostic helpers (spec §§6.8, 6.9; plan 048).

The MVP handshake surfaces only one flavor of diagnostic: a protocol error
that stops analysis before any target-level result is produced. Later plans
(dependency, symbol) will attach per-target diagnostics through the same
shape; keeping the record here means the handshake path already knows how
to structure them.
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

/-- Repository-relative location for a diagnostic (spec §6.3). Only the
    `path` is populated for module-map errors; `start` / `end` slots are
    reserved for later plans (dependency, symbol) that resolve source
    ranges. `path` is a `/`-separated repo-relative POSIX path. -/
structure Location where
  path : String
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
