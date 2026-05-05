# Examples

Pre-computed REPL command sequences. Pipe one into the runner with
`--repl` and the simulation will pause between ticks and apply each
command in order.

## zombie-in-office

A *Scribblenauts*-style test: a dull office runs for 20 ticks, then
an injected `Spawn` + `Equip` + `QueueTask` sequence drops a zombie
with a steel crowbar into the middle of the floor. The zombie
pathfinds to alice and beats her to death over the next 8 ticks.

```
cargo run --bin fortress -- office --repl < examples/zombie-in-office.txt
```

What this demonstrates:

- The injection surface (`Action::Spawn`, `Action::QueueTask`) is
  enough to introduce a fully-equipped hostile creature with goals,
  no scenario edits required.
- Library role + item templates ("zombie", "steel crowbar") apply
  cleanly: full humanoid body plan, MainHand equipment, attack works
  out of the box.
- Combat resolves naturally: alice loses body parts in sequence
  (ear, leg, torso, hand, neck) until she's killed.

What it surfaces as missing — i.e. the next systems to build:

- The other workers continue their routine completely obliviously
  while alice is bludgeoned to death five feet away. We need
  **perception** + **sound emission** so combat noise alerts other
  workers, who can then **flee** or **hide**.
- The gossip system still emits "alice catches bob's eye" after
  alice is dead, because it filters by `With<Worker>` rather than
  alive workers. Tiny scenario fix; also a hint that engine helpers
  for "living members of faction X" would prevent this class of bug.

See `docs/EMERGENCE.md` for the per-scenario design notes.
