# CRDT Parity Checklists

Last reviewed: 2026-05-14

This directory tracks parity work by CRDT element and supporting CRDT data
structure. Each checklist should stay small enough to be useful during porting:
reference files, current Rust status, covered behavior, gaps, and recommended
next tests.

Use these status labels:

- `covered`: implemented and tested in Rust.
- `partial`: implemented or tested only for a narrow slice.
- `blocked`: depends on another layer that is not ready yet.
- `missing`: not implemented or not tested.

## Checklists

- [Element and Metadata](element.md)
- [Primitive](primitive.md)
- [Counter](counter.md)
- [Object and ElementRHT](object-element-rht.md)
- [Array and RGATreeList](array-rga-tree-list.md)
- [Text and RGATreeSplit](text-rga-tree-split.md)
- [Tree](tree.md)
- [Splay and Index](splay-index.md)
- [RHT Attributes](rht-attributes.md)
- [Root and Garbage Collection](root-and-gc.md)
- [Upstream Skipped Tests](upstream-skipped-tests.md)

## Maintenance Rules

- Update the relevant checklist whenever a CRDT behavior is added, tested, or
  intentionally deferred.
- Keep implementation-source references in these docs rather than in Rust code
  comments.
- Prefer one checklist update per implementation commit so future agents can
  see what changed and what remains.
- Keep broad summaries in `docs/current-porting-gaps.md`; keep detailed test
  matrices and itemized parity status here.
- Keep skipped upstream tests in `upstream-skipped-tests.md`. Do not turn a
  skipped JS/Go CRDT case into a Rust pass target without documenting the
  upstream status change or an explicit Rust divergence.
