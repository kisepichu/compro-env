/-!
# Monoid

Structural interface for an associative binary operation with a two-sided
identity, mirroring the Rust and C++ library counterparts.
-/

namespace Algebra

structure Monoid (T : Type) where
  id : T
  op : T → T → T
  identityLeft  : ∀ x, op id x = x
  identityRight : ∀ x, op x id = x
  assoc         : ∀ a b c, op (op a b) c = op a (op b c)

def addMonoid : Monoid Int where
  id := 0
  op := (· + ·)
  identityLeft  := by intro x; simp
  identityRight := by intro x; simp
  assoc         := by intro a b c; simp [Int.add_assoc]

end Algebra
