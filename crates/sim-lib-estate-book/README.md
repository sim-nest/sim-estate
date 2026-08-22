# sim-lib-estate-book

Immutable, hash-linked estate operation history over a minimal Table/Dir CAS port.
Events are published before run heads advance, so every acknowledged head is
rebuildable and conflicts preserve both writers' immutable envelopes.
