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
  deriving Repr, DecidableEq

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

end Analyzer.Diagnostics
