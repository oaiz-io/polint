package uuid

// UUID is a parsed identifier.
type UUID [16]byte

func Parse(s string) (UUID, error) { return UUID{}, nil }

func (u UUID) String() string { return "" }
