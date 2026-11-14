# Design Documents

New design documents are based on [TEMPLATE.md](TEMPLATE.md) and listed here.

## Contents

None yet.

The contract this SDK is built against is written elsewhere under `docs/` and
stays there: the porting rules, the dependency boundaries, the CRDT parity
checklists and the known gaps are indexed in [../README.md](../README.md), and
the milestone plans written so far are under [../plans/](../plans/).

## Guidelines

Write a design document before implementing a change to the architecture: a
new crate or module boundary, a change to the wire format, or behavior that
deliberately differs from the JS SDK. A port that follows the JS SDK needs
none; its design is the JS SDK's, and
[../porting-from-js-sdk.md](../porting-from-js-sdk.md) says how to follow it.

A design document goes through the same pull request flow as code
([CONTRIBUTING.md](../../CONTRIBUTING.md)).
