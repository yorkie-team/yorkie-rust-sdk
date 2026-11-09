/*
 * Generates the Text undo interop fixtures the Rust SDK checks in
 * crates/yorkie-protocol/tests/text_undo_interop.rs. Not built by cargo.
 *
 * Run it from a scratch copy of yorkie-js-sdk (Node 22, pnpm 9), never from
 * a checkout you work in:
 *
 *   cp tools/interop/text_undo_fixtures_test.ts \
 *     <js-sdk>/packages/sdk/test/unit/api/
 *   cd <js-sdk>
 *   TEXT_UNDO_FIXTURE_DIR=<rust-sdk>/crates/yorkie-protocol/tests/fixtures/text_undo \
 *     pnpm sdk exec vitest run test/unit/api/text_undo_fixtures_test.ts
 *
 * The name ends in `_test.ts` because vitest collects only such files there.
 * tools/interop/text_undo_fixtures_test.go builds the same scenarios in Go.
 *
 * Every map holds at most one entry (one attribute, one actor per version
 * vector, or actors inserted in sorted order): Go marshals map entries in
 * random order, and the Rust decoder sorts them, so byte equality holds only
 * when the order cannot differ.
 */

import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, it, assert } from 'vitest';
import { toBinary } from '@bufbuild/protobuf';
import { converter } from '@yorkie-js/sdk/src/api/converter';
import { ChangePackSchema as PbChangePackSchema } from '@yorkie-js/sdk/src/api/yorkie/v1/resources_pb';
import { Document } from '@yorkie-js/sdk/src/document/document';
import { ChangePack } from '@yorkie-js/sdk/src/document/change/change_pack';
import { Checkpoint } from '@yorkie-js/sdk/src/document/change/checkpoint';
import {
  InitialVersionVector,
  VersionVector,
} from '@yorkie-js/sdk/src/document/time/version_vector';
import { Text } from '@yorkie-js/sdk/src/yorkie';
import { maxVectorOf } from '@yorkie-js/sdk/test/helper/helper';

const A1 = '000000000000000000000001';
const A2 = '000000000000000000000002';
const KEY = 'text-undo-interop';

/**
 * `write` stores the given pack as lowercase hex, one line, and the sorted
 * JSON of the document JS built, when the fixture directory is set.
 */
function write(name: string, pack: ChangePack<any>, json: string): string {
  const hex = converter.bytesToHex(
    toBinary(PbChangePackSchema, converter.toChangePack(pack)),
  );
  const dir = process.env.TEXT_UNDO_FIXTURE_DIR;
  if (dir) {
    writeFileSync(join(dir, `${name}.hex`), `${hex}\n`);
    writeFileSync(join(dir, `${name}.json`), `${json}\n`);
  }
  return hex;
}

/**
 * `crossSync` delivers each replica's pending changes to the other and
 * acknowledges them, without a server and without moving GC forward (as in
 * text_restore_after_gc_test.ts).
 */
function crossSync<T>(d1: Document<T>, d2: Document<T>): void {
  const p1 = d1.createChangePack();
  const p2 = d2.createChangePack();
  const lastSeq = (from: ChangePack<any>) => {
    const changes = from.getChanges();
    return changes.length
      ? changes[changes.length - 1].getID().getClientSeq()
      : 0;
  };
  type Changes = ReturnType<ChangePack<any>['getChanges']>;
  const pack = (clientSeq: number, changes: Changes) =>
    ChangePack.create(
      KEY,
      Checkpoint.of(0n, clientSeq),
      false,
      changes,
      InitialVersionVector,
    );
  d2.applyChangePack(pack(0, p1.getChanges()));
  d1.applyChangePack(pack(0, p2.getChanges()));
  d1.applyChangePack(pack(lastSeq(p1), []));
  d2.applyChangePack(pack(lastSeq(p2), []));
}

describe('Text undo interop fixtures', () => {
  // (a) A pack of one replica's changes: a deletion, its undo (restore
  // mode), its redo (retombstone mode), then a replace whose undo carries
  // both span lists, and that undo's redo.
  it('writes restore_retombstone_pack', () => {
    const doc = new Document<{ t: Text }>(KEY);
    doc.setActor(A1);
    doc.update((r) => {
      r.t = new Text();
      r.t.edit(0, 0, '0123456789', { b: '1' });
    });
    doc.update((r) => r.t.edit(2, 5, ''));
    assert.equal(doc.getRoot().t.toString(), '0156789');
    doc.history.undo();
    assert.equal(doc.getRoot().t.toString(), '0123456789');
    doc.history.redo();
    assert.equal(doc.getRoot().t.toString(), '0156789');
    doc.update((r) => r.t.edit(0, 2, 'ab'));
    assert.equal(doc.getRoot().t.toString(), 'ab56789');
    doc.history.undo();
    assert.equal(doc.getRoot().t.toString(), '0156789');
    doc.history.redo();
    assert.equal(doc.getRoot().t.toString(), 'ab56789');

    const changes = doc.createChangePack().getChanges();
    const pack = ChangePack.create(
      KEY,
      Checkpoint.of(BigInt(changes.length), 0),
      false,
      changes,
      doc.getVersionVector(),
    );
    write('restore_retombstone_pack', pack, doc.toSortedJSON());

    // A second JS replica reads the same pack to the same text.
    const peer = new Document<{ t: Text }>(KEY);
    peer.setActor(A2);
    peer.applyChangePack(
      converter.fromChangePack(converter.toChangePack(pack)),
    );
    assert.equal(peer.getRoot().t.toString(), 'ab56789');
    assert.equal(peer.toSortedJSON(), doc.toSortedJSON());
  });

  // (b) A snapshot whose text holds `cd`, removed by A1, restored by A1's
  // undo, then removed by A2, and `ef`, recreated by A1's undo after GC
  // purged it, with the lamport-0 stand-in removal and its cancel.
  it('writes ledger_snapshot_pack', () => {
    const d1 = new Document<{ t: Text }>(KEY);
    const d2 = new Document<{ t: Text }>(KEY);
    d1.setActor(A1);
    d2.setActor(A2);
    d1.update((r) => {
      r.t = new Text();
      r.t.edit(0, 0, 'abcdefgh', { b: '1' });
    });
    crossSync(d1, d2);

    d1.update((r) => r.t.edit(4, 6, ''));
    crossSync(d1, d2);
    assert.isAbove(d1.garbageCollect(maxVectorOf([A1])), 0);
    d1.history.undo();
    crossSync(d1, d2);
    assert.equal(d1.getRoot().t.toString(), 'abcdefgh');

    d1.update((r) => r.t.edit(2, 4, ''));
    crossSync(d1, d2);
    d1.history.undo();
    crossSync(d1, d2);
    d2.update((r) => r.t.edit(2, 4, ''));
    crossSync(d1, d2);
    assert.equal(d1.getRoot().t.toString(), 'abefgh');
    assert.equal(d2.toSortedJSON(), d1.toSortedJSON());

    const vector = d1.getVersionVector();
    const sorted = new VersionVector(
      new Map([
        [A1, vector.get(A1)!],
        [A2, vector.get(A2)!],
      ]),
    );
    const pack = ChangePack.create(
      KEY,
      Checkpoint.of(1n, 0),
      false,
      [],
      sorted,
      converter.snapshotToBytes(d1.getRootObject(), new Map()),
    );
    write('ledger_snapshot_pack', pack, d1.toSortedJSON());
  });

  // (c) A snapshot whose text holds the tombstone the undo of a style left
  // on `bold`: `toTextNodes` writes its `isRemoved` (#1364).
  it('writes removed_text_attr_snapshot_pack', () => {
    const doc = new Document<{ t: Text }>(KEY);
    doc.setActor(A1);
    doc.update((r) => {
      r.t = new Text();
      r.t.edit(0, 0, 'hello');
    });
    doc.update((r) => r.t.setStyle(0, 5, { bold: 'true' }));
    doc.history.undo();
    assert.equal(doc.toSortedJSON(), '{"t":[{"val":"hello"}]}');
    assert.equal(doc.getGarbageLen(), 1);

    const pack = ChangePack.create(
      KEY,
      Checkpoint.of(1n, 0),
      false,
      [],
      doc.getVersionVector(),
      converter.snapshotToBytes(doc.getRootObject(), new Map()),
    );
    write('removed_text_attr_snapshot_pack', pack, doc.toSortedJSON());

    const peer = new Document<{ t: Text }>(KEY);
    peer.setActor(A2);
    peer.applyChangePack(
      converter.fromChangePack(converter.toChangePack(pack)),
    );
    assert.equal(peer.toSortedJSON(), doc.toSortedJSON());
    assert.equal(peer.getGarbageLen(), 1);
  });
});
