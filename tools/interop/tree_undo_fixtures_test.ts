/*
 * Generates the Tree undo interop fixtures the Rust SDK checks in
 * crates/yorkie-protocol/tests/tree_undo_interop.rs. Not built by cargo.
 *
 * Run it from a scratch copy of yorkie-js-sdk (Node 22, pnpm 9), never from
 * a checkout you work in:
 *
 *   cp tools/interop/tree_undo_fixtures_test.ts \
 *     <js-sdk>/packages/sdk/test/unit/api/
 *   cd <js-sdk>
 *   TREE_UNDO_FIXTURE_DIR=<rust-sdk>/crates/yorkie-protocol/tests/fixtures/tree_undo \
 *     pnpm sdk exec vitest run test/unit/api/tree_undo_fixtures_test.ts
 *
 * The name ends in `_test.ts` because vitest collects only such files there.
 *
 * Every map holds at most one entry (one attribute per element, one actor
 * per version vector): Go marshals map entries in random order, and the
 * Rust decoder sorts them, so byte equality holds only when the order
 * cannot differ.
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
import { Tree } from '@yorkie-js/sdk/src/yorkie';

const A1 = '000000000000000000000001';
const A2 = '000000000000000000000002';
const KEY = 'tree-undo-interop';

/**
 * `write` stores the given pack as lowercase hex, one line, and the XML
 * after each change, one per line, when the fixture directory is set.
 */
function write(name: string, pack: ChangePack<any>, xmls: Array<string>) {
  const hex = converter.bytesToHex(
    toBinary(PbChangePackSchema, converter.toChangePack(pack)),
  );
  const dir = process.env.TREE_UNDO_FIXTURE_DIR;
  if (dir) {
    writeFileSync(join(dir, `${name}.hex`), `${hex}\n`);
    writeFileSync(join(dir, `${name}.json`), `${JSON.stringify(xmls)}\n`);
  }
}

/**
 * `record` runs each step on a fresh A1 document, keeps the XML after each
 * change, writes the pack of all its changes, and checks that a second JS
 * replica reads the pack to the same XML.
 */
function record(
  name: string,
  steps: Array<(doc: Document<{ t: Tree }>) => void>,
): Array<string> {
  const doc = new Document<{ t: Tree }>(KEY);
  doc.setActor(A1);
  const xmls = steps.map((step) => {
    step(doc);
    return doc.getRoot().t.toXML();
  });

  const changes = doc.createChangePack().getChanges();
  assert.equal(changes.length, steps.length);
  const pack = ChangePack.create(
    KEY,
    Checkpoint.of(BigInt(changes.length), 0),
    false,
    changes,
    doc.getVersionVector(),
  );
  write(name, pack, xmls);

  const peer = new Document<{ t: Tree }>(KEY);
  peer.setActor(A2);
  peer.applyChangePack(converter.fromChangePack(converter.toChangePack(pack)));
  assert.equal(peer.getRoot().t.toXML(), xmls[xmls.length - 1]);
  return xmls;
}

describe('Tree undo interop fixtures', () => {
  // (a) A deletion inside a paragraph, its undo (restore mode) and redo
  // (retombstone mode), then a replace of a styled paragraph whose undo
  // carries both span lists, and that undo's redo.
  it('writes restore_retombstone_pack', () => {
    const xmls = record('restore_retombstone_pack', [
      (doc) =>
        doc.update((r) => {
          r.t = new Tree({
            type: 'doc',
            children: [
              { type: 'p', children: [{ type: 'text', value: 'abcd' }] },
              {
                type: 'p',
                attributes: { a: '1' },
                children: [{ type: 'text', value: 'ef' }],
              },
            ],
          });
        }),
      (doc) => doc.update((r) => r.t.edit(2, 4)),
      (doc) => doc.history.undo(),
      (doc) => doc.history.redo(),
      (doc) =>
        doc.update((r) =>
          r.t.edit(4, 8, {
            type: 'p',
            children: [{ type: 'text', value: 'X' }],
          }),
        ),
      (doc) => doc.history.undo(),
      (doc) => doc.history.redo(),
    ]);
    assert.deepEqual(xmls, [
      '<doc><p>abcd</p><p a="1">ef</p></doc>',
      '<doc><p>ad</p><p a="1">ef</p></doc>',
      '<doc><p>abcd</p><p a="1">ef</p></doc>',
      '<doc><p>ad</p><p a="1">ef</p></doc>',
      '<doc><p>ad</p><p>X</p></doc>',
      '<doc><p>ad</p><p a="1">ef</p></doc>',
      '<doc><p>ad</p><p>X</p></doc>',
    ]);
  });

  // (b) A merge of two paragraphs, its undo (a split under tickets the undo
  // issued, carried as split_tickets), and that undo's redo.
  it('writes merge_undo_pack', () => {
    const xmls = record('merge_undo_pack', [
      (doc) =>
        doc.update((r) => {
          r.t = new Tree({
            type: 'doc',
            children: [
              { type: 'p', children: [{ type: 'text', value: 'ab' }] },
              { type: 'p', children: [{ type: 'text', value: 'cd' }] },
            ],
          });
        }),
      (doc) => doc.update((r) => r.t.edit(3, 5)),
      (doc) => doc.history.undo(),
      (doc) => doc.history.redo(),
    ]);
    assert.deepEqual(xmls, [
      '<doc><p>ab</p><p>cd</p></doc>',
      '<doc><p>abcd</p></doc>',
      '<doc><p>ab</p><p>cd</p></doc>',
      '<doc><p>abcd</p></doc>',
    ]);
  });
});
