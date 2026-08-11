import Init
def hello : Nat := 42
theorem hello_eq : hello = 42 := rfl
axiom mystery : Nat
opaque secret : Nat
abbrev Alias := Nat
