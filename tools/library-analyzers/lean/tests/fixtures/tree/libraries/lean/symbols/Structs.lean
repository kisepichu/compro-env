import Init
structure Point where
  x : Nat
  y : Nat
class HasFoo (α : Type) where
  foo : α
instance : HasFoo Nat where
  foo := 0
inductive Tree where
  | leaf
  | node (l r : Tree)
