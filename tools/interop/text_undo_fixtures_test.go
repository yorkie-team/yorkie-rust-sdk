/*
 * Builds the Text undo interop scenarios of text_undo_fixtures_test.ts with
 * the Go SDK, for crates/yorkie-protocol/tests/text_undo_interop.rs. Not
 * built by cargo.
 *
 * Go writes the change pack of the restore and retombstone scenario byte
 * for byte as JS does, so this only checks it against the JS fixture. Go
 * writes a different ledger snapshot: it stamps the attribute of a piece an
 * undo recreates with the undo's ticket, where JS keeps the ticket of the
 * insertion. This writes that snapshot as go_ledger_snapshot_pack.hex and
 * checks that it still differs from the JS one.
 *
 * Run it from a scratch copy of yorkie (at 54b96762 or later), never from a
 * checkout you work in:
 *
 *   cp tools/interop/text_undo_fixtures_test.go <yorkie>/pkg/document/
 *   cd <yorkie>
 *   TEXT_UNDO_FIXTURE_DIR=<rust-sdk>/crates/yorkie-protocol/tests/fixtures/text_undo \
 *     go test ./pkg/document -run TestTextUndoInteropFixtures -count=1
 *
 * Go marshals map entries in random order, so this marshals
 * deterministically (sorted keys), which is the order the JS generator
 * inserts and the Rust encoder writes.
 */

package document_test

import (
	"encoding/hex"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/stretchr/testify/require"
	"google.golang.org/protobuf/proto"

	"github.com/yorkie-team/yorkie/api/converter"
	"github.com/yorkie-team/yorkie/pkg/document"
	"github.com/yorkie-team/yorkie/pkg/document/change"
	"github.com/yorkie-team/yorkie/pkg/document/json"
	"github.com/yorkie-team/yorkie/pkg/document/presence"
	"github.com/yorkie-team/yorkie/pkg/document/time"
	"github.com/yorkie-team/yorkie/test/helper"
)

const interopKey = "text-undo-interop"

// fixtureDir returns the fixture directory, skipping the test without one.
func fixtureDir(t *testing.T) string {
	t.Helper()

	dir := os.Getenv("TEXT_UNDO_FIXTURE_DIR")
	if dir == "" {
		t.Skip("TEXT_UNDO_FIXTURE_DIR is not set")
	}
	return dir
}

// packHex marshals the given pack deterministically, as lowercase hex.
func packHex(t *testing.T, pack *change.Pack) string {
	t.Helper()

	pbPack, err := converter.ToChangePack(pack)
	require.NoError(t, err)
	bytes, err := proto.MarshalOptions{Deterministic: true}.Marshal(pbPack)
	require.NoError(t, err)
	return hex.EncodeToString(bytes)
}

// jsFixture reads the hex of the JS fixture of the given name.
func jsFixture(t *testing.T, dir, name string) string {
	t.Helper()

	want, err := os.ReadFile(filepath.Join(dir, name+".hex"))
	require.NoError(t, err)
	return strings.TrimSpace(string(want))
}

func interopActor(t *testing.T, hexID string) time.ActorID {
	t.Helper()

	actor, err := time.ActorIDFromHex(hexID)
	require.NoError(t, err)
	return actor
}

func interopEdit(t *testing.T, doc *document.Document, from, to int, content string) {
	t.Helper()

	require.NoError(t, doc.Update(func(root *json.Object, _ *presence.Presence) error {
		root.GetText("t").Edit(from, to, content)
		return nil
	}))
}

// interopNewText creates the text with its first insertion in one change,
// as the JS generator does. JS stores an attribute value JSON-encoded.
func interopNewText(t *testing.T, doc *document.Document, content string) {
	t.Helper()

	require.NoError(t, doc.Update(func(root *json.Object, _ *presence.Presence) error {
		root.SetNewText("t").Edit(0, 0, content, map[string]string{"b": `"1"`})
		return nil
	}))
}

func TestTextUndoInteropFixtures(t *testing.T) {
	a1 := interopActor(t, "000000000000000000000001")
	a2 := interopActor(t, "000000000000000000000002")

	t.Run("restore_retombstone_pack", func(t *testing.T) {
		doc := document.New(interopKey)
		doc.SetActor(a1)
		interopNewText(t, doc, "0123456789")
		interopEdit(t, doc, 2, 5, "")
		require.NoError(t, doc.Undo())
		require.Equal(t, "0123456789", textOf(t, doc, "t").String())
		require.NoError(t, doc.Redo())
		interopEdit(t, doc, 0, 2, "ab")
		require.NoError(t, doc.Undo())
		require.Equal(t, "0156789", textOf(t, doc, "t").String())
		require.NoError(t, doc.Redo())
		require.Equal(t, "ab56789", textOf(t, doc, "t").String())

		dir := fixtureDir(t)
		changes := doc.CreateChangePack().Changes
		require.Equal(t, jsFixture(t, dir, "restore_retombstone_pack"), packHex(t, change.NewPack(
			interopKey,
			change.NewCheckpoint(int64(len(changes)), 0),
			changes,
			doc.VersionVector(),
			nil,
		)))
	})

	t.Run("ledger_snapshot_pack", func(t *testing.T) {
		d1 := document.New(interopKey)
		d2 := document.New(interopKey)
		d1.SetActor(a1)
		d2.SetActor(a2)
		interopNewText(t, d1, "abcdefgh")
		crossSync(t, d1, d2)

		interopEdit(t, d1, 4, 6, "")
		crossSync(t, d1, d2)
		require.Positive(t, d1.GarbageCollect(helper.MaxVersionVector(a1)))
		require.NoError(t, d1.Undo())
		crossSync(t, d1, d2)
		require.Equal(t, "abcdefgh", textOf(t, d1, "t").String())

		interopEdit(t, d1, 2, 4, "")
		crossSync(t, d1, d2)
		require.NoError(t, d1.Undo())
		crossSync(t, d1, d2)
		interopEdit(t, d2, 2, 4, "")
		crossSync(t, d1, d2)
		require.Equal(t, "abefgh", textOf(t, d1, "t").String())
		require.Equal(t, d1.Marshal(), d2.Marshal())

		snapshot, err := converter.SnapshotToBytes(d1.RootObject(), nil)
		require.NoError(t, err)
		dir := fixtureDir(t)
		got := packHex(t, change.NewPack(
			interopKey,
			change.NewCheckpoint(1, 0),
			nil,
			d1.VersionVector(),
			snapshot,
		))
		// If this fails, Go keeps the insertion's attribute ticket as JS
		// does, and the Go fixture can go.
		require.NotEqual(t, jsFixture(t, dir, "ledger_snapshot_pack"), got)
		require.NoError(t, os.WriteFile(
			filepath.Join(dir, "go_ledger_snapshot_pack.hex"), []byte(got+"\n"), 0o644,
		))
	})
}
