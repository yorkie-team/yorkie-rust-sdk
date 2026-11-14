// What THIS repository's harness configuration must say.
//
// `scripts/agent/*.test.mjs` grade the harness code against a fixture
// configuration and are the same bytes in every repository that runs the
// harness. This file is the other half: it grades the values in this directory
// and the toolchain action — the files that are different in every repository
// — against the repository itself, and it is the only test that reads them.
// `config.json` cannot carry comments, so the reason each entry exists is
// written here, beside the assertion that pins it.
//
// Run from `scripts/agent`: `npm run test:repo`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.join(HERE, "..", "..");
const AGENT = path.join(ROOT, "scripts", "agent");
const WORKFLOWS = path.join(ROOT, ".github", "workflows");

// This directory, whatever the caller's environment says. Set before the first
// harness module is imported — which is why the imports below are dynamic: a
// static import would be evaluated before this line.
process.env.AGENT_HARNESS_CONFIG_DIR = HERE;

const agent = (file) => import(path.join(AGENT, file));
const checks = await agent("checks.mjs");
const panel = await agent("review-panel.mjs");
const rounds = await agent("rounds.mjs");
const fixGuard = await agent("fix-guard.mjs");
const testRemovals = await agent("test-removals.mjs");
const cargoRules = await agent("cargo-rules.mjs");
const reviewScope = await agent("review-scope.mjs");
const { compileGlob, loadConfig } = await agent("config.mjs");
const workflowValues = await agent("workflow-values.mjs");
const manifest = await agent("manifest.mjs");

const CONFIG = loadConfig();

const read = (...parts) => readFileSync(path.join(ROOT, ...parts), "utf8");
/** A file's lines that are not comments: prose does not run. */
const codeOf = (text) => text.split("\n").filter((l) => !/^\s*#/.test(l)).join("\n");
const TRACKED = execFileSync("git", ["ls-files"], { cwd: ROOT, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 })
  .split("\n")
  .filter(Boolean);
const isTracked = (file) => TRACKED.includes(file);
const matching = (glob) => {
  const re = compileGlob(glob);
  return TRACKED.filter((f) => re.test(f));
};

const CI = read(checks.CI_WORKFLOW_PATH);
const MAKEFILE = read("Makefile");
const ACTION = read(".github", "actions", "agent-toolchain", "action.yml");
const ACTION_CODE = codeOf(ACTION);

/** One job of `ci.yml`, from its key to the next job's. */
function ciJob(id) {
  const start = CI.indexOf(`\n  ${id}:\n`);
  assert.ok(start >= 0, `ci.yml no longer has a \`${id}\` job`);
  const rest = CI.slice(start + 1);
  const next = rest.slice(1).search(/^ {2}[a-z_-]+:\s*$/m);
  return next < 0 ? rest : rest.slice(0, next + 1);
}
/** The single-line `run:` commands of a job, in order. */
const runLines = (job) => [...codeOf(job).matchAll(/^\s+(?:- )?run: (?!\|)(.+)$/gm)].map((m) => m[1].trim());
/** A Makefile target's prerequisites and recipe lines. */
function makeTarget(name) {
  const m = new RegExp(`^${name}:(.*)\\n((?:\\t.*\\n)*)`, "m").exec(MAKEFILE);
  assert.ok(m, `the Makefile has no \`${name}\` target`);
  return { needs: m[1].trim().split(/\s+/).filter(Boolean), recipe: m[2].split("\n").map((l) => l.trim()).filter(Boolean) };
}
/** The workspace members `Cargo.toml` lists. */
const MEMBERS = [.../members = \[([^\]]*)\]/.exec(read("Cargo.toml"))[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
/** `channel` in rust-toolchain.toml: the version every other pin must equal. */
const RUST_VERSION = /^channel = "(\d+\.\d+\.\d+)"$/m.exec(read("rust-toolchain.toml"))?.[1];

test("this file grades the configuration in its own directory", () => {
  assert.equal(loadConfig(HERE), CONFIG);
  assert.equal(CONFIG.repo.name, "yorkie-rust-sdk");
  assert.ok(TRACKED.length > 150, `expected the repository's files, git listed ${TRACKED.length}`);
  assert.deepEqual(MEMBERS, ["crates/yorkie", "crates/yorkie-client", "crates/yorkie-core", "crates/yorkie-protocol", "tools/protogen"]);
  assert.match(String(RUST_VERSION), /^\d+\.\d+\.\d+$/, "rust-toolchain.toml no longer pins `channel` to one version");
});

// --- where the shared files came from ---------------------------------------------

test("the shared files are a copy of yorkie-team/yorkie", () => {
  // The core cannot know which repository is the origin. A copy whose
  // `upstream.json` was edited to `{ "repository": null }` would pass every
  // drift check, because that is what the origin's says.
  const upstream = JSON.parse(read(".github", "agent-harness", "upstream.json"));
  assert.equal(upstream.repository, "yorkie-team/yorkie");
  assert.match(upstream.commit, /^[0-9a-f]{40}$/);
  assert.match(upstream.manifestDigest, /^[0-9a-f]{64}$/);
});

test("every file the issue-to-PR prompt names exists here", async () => {
  // The prompt is shared, so it names the same paths in every repository: the
  // two process documents, the task directory, the design template and index,
  // the pull request template.
  const { IMPLEMENT_PROMPT_PATHS, missingImplementPaths } = await agent("conventions.mjs");
  assert.ok(IMPLEMENT_PROMPT_PATHS.length >= 6, "the list was emptied");
  assert.deepEqual(missingImplementPaths(ROOT), []);
  // …and each is TRACKED, not merely on this machine's disk.
  for (const entry of IMPLEMENT_PROMPT_PATHS) {
    const hit = execFileSync("git", ["ls-files", "--", entry.path], { cwd: ROOT, encoding: "utf8" }).trim();
    assert.ok(hit !== "", `${entry.path} exists here but git does not track it`);
  }
});

test("no checkout converts the shared files' line endings", () => {
  // The manifest is over bytes. With `core.autocrlf` on and no attribute, git
  // rewrites every line ending on checkout and each shared file then hashes
  // differently.
  const lines = read(".gitattributes").split("\n").map((l) => l.trim());
  for (const line of manifest.GITATTRIBUTES_LINES) assert.ok(lines.includes(line), `.gitattributes must contain \`${line}\``);
  if (!existsSync(path.join(ROOT, ".git"))) return;
  for (const rel of ["scripts/agent/config.mjs", "scripts/agent/lenses/lenses.json", ".github/workflows/agent-fix.yml"]) {
    const out = execFileSync("git", ["check-attr", "text", "--", rel], { cwd: ROOT, encoding: "utf8" });
    assert.match(out, /: text: unset\s*$/, `${rel}: ${out.trim()}`);
  }
  // The package's own dependencies are installed beside it and never tracked.
  assert.ok(read(".gitignore").split("\n").includes("scripts/agent/node_modules/"));
});

// --- what the harness assumes of this repository's CI workflow ---------------------

test("the CI workflow is ci.yml, and its name is exactly CI", () => {
  // The panel and the CI-fix arm start on `workflow_run` with
  // `workflows: ["CI"]`, which matches the NAME. With any other name they
  // never start: no error, no run. It was `ci` before the harness arrived.
  assert.equal(checks.CI_WORKFLOW_PATH, ".github/workflows/ci.yml");
  assert.match(CI, /^name: CI$/m, "ci.yml must be the workflow whose runs are named CI, capitals included");
  assert.equal([...CI.matchAll(/^name:/gm)].length, 1);
});

test("ci.yml files one run for every PR head, whatever the PR touches", () => {
  // A workflow skipped by a trigger-level path filter files NO run, so there
  // is no `workflow_run` event and neither the panel nor the CI-fix arm
  // starts: a docs-only PR would sit outside the loop. And two triggers that
  // can both fire for one PR head would make "the newest run" arbitrary.
  const on = codeOf(CI.slice(CI.indexOf("\non:"), CI.indexOf("\npermissions:")));
  const events = [...on.matchAll(/^ {2}([a-z_]+):/gm)].map((m) => m[1]);
  assert.deepEqual(events.sort(), ["pull_request", "push"], "a new CI trigger may fire for a PR head SHA");
  assert.match(on, /^ {2}push:\n {4}branches: \[main\]$/m, "push must stay restricted to main, or every PR branch push races pull_request");
  assert.match(on, /^ {2}pull_request:\s*$/m, "pull_request must stay unfiltered");
  assert.ok(!/paths(-ignore)?:/.test(CI), "ci.yml must not filter by path: a filtered-out PR files no run");
  // No job is conditional either, so "CI is green" means every lane ran.
  assert.ok(!/^ {4}if:/m.test(CI), "a job-level `if:` in ci.yml makes a lane optional; say so in coverage-note.md first");
});

test("ci.laneReports says what ci.yml uploads", () => {
  // This repository's CI uploads no per-lane report, so the CI-fix arm reads
  // the failed step's log tail.
  assert.equal(CONFIG.ci.laneReports, /^\s+name: ci-lane-reports\s*$/m.test(CI));
  assert.equal(CONFIG.ci.laneReports, false);
});

// --- the paths that define CI -------------------------------------------------------
//
// `ci.definingPaths`, entry by entry, with the reason each is on the list. The
// harness adds the entries every repository shares (the workflows and actions,
// CODEOWNERS, this directory, the hook layer) around these. A pull request
// that touches any of them is never promoted automatically.
const CI_DEFINING = [
  // `ci.yml`'s check job runs `make fmt-check`, `make clippy`, `make test` and
  // `make doc`; what those do is the Makefile.
  "Makefile",
  // The dependency graph, the features and the targets cargo builds — and,
  // through dependencies, the build scripts and procedural macros that RUN
  // when anything is compiled. A manifest can also switch a test target off.
  "Cargo.toml",
  "crates/*/Cargo.toml",
  "tools/*/Cargo.toml",
  "Cargo.lock",
  // The compiler. In `ci.yml` the workspace's toolchain file wins over the
  // default the setup step installs, so a branch editing it chooses what
  // checks the branch.
  "rust-toolchain.toml",
  // The generator the freshness test compares the checked-in protobuf code
  // against: a branch that edits it redefines "fresh".
  "tools/protogen/**",
  // GUARDS FOR FILES THAT DO NOT EXIST YET (see RESERVED below). Creating any
  // of them changes what CI executes or checks:
  //   - the legacy spelling of the toolchain file, which rustup also reads;
  "rust-toolchain",
  //   - cargo configuration: a compiler wrapper, a runner, an alias, `[env]`,
  //     a replaced registry;
  ".cargo/**",
  //   - a build script, which runs at compile time. No crate has one today;
  "**/build.rs",
  //   - rustfmt and clippy configuration, which decide what those lanes accept.
  "rustfmt.toml",
  ".rustfmt.toml",
  "clippy.toml",
  ".clippy.toml",
];

test("ci.definingPaths covers the surface CI's behaviour is read from", () => {
  assert.deepEqual([...CONFIG.ci.definingPaths].sort(), [...CI_DEFINING].sort(), "config.json and the annotated list above have drifted");

  const defining = [
    ".github/workflows/ci.yml",
    ".github/actions/agent-toolchain/action.yml",
    ".github/agent-harness/config.json",
    "Makefile",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "rust-toolchain",
    "tools/protogen/src/lib.rs",
    "tools/protogen/tests/up_to_date.rs",
    ".cargo/config.toml",
    "build.rs",
    "crates/yorkie-core/build.rs",
    "rustfmt.toml",
    "clippy.toml",
    // Every workspace member's manifest, read from the workspace's own list:
    // a member added under a new directory fails here until it is covered.
    ...MEMBERS.map((m) => `${m}/Cargo.toml`),
  ];
  for (const p of defining) assert.equal(checks.definesCi(p), true, `${p} defines what CI does`);
  // What the check job runs is the Makefile and nothing else a branch could
  // supply: no script, no other file.
  for (const cmd of runLines(ciJob("check"))) assert.match(cmd, /^make [a-z-]+$/, `ci.yml's check job runs \`${cmd}\`, which is not a make target`);

  // …and it does not swallow the code under review. A gate that refuses every
  // PR is the same outage as one that refuses none.
  for (const p of [
    "crates/yorkie-core/src/crdt/tree.rs",
    "crates/yorkie/tests/integration_tree.rs",
    "crates/yorkie-protocol/proto/yorkie/v1/resources.proto",
    "crates/yorkie-protocol/src/generated/yorkie.v1.rs",
    "tools/interop/text_undo_fixtures_test.ts",
    "docs/current-porting-gaps.md",
    "README.md",
  ]) {
    assert.equal(checks.definesCi(p), false, `${p} is code or prose under review, not the CI definition`);
  }
});

test("every configured path and glob matches a tracked file, or is reserved with a reason", () => {
  // The loader can tell a glob from a non-glob, not a right glob from a wrong
  // one: `crates/tests` matches only a FILE of that name, `*.rs` only at the
  // root, and a renamed directory leaves a valid glob behind. On these lists
  // "matches nothing" means an entry quietly off a refusal surface or out of a
  // review scope. So every entry must match a tracked file — or be on
  // RESERVED, listed AHEAD of the file existing so that creating it is
  // refused by the ready gate and reviewed as policy. The day one of them is
  // created this fails, and whoever created it decides what it now needs.
  const RESERVED = {
    // rustup reads `rust-toolchain` as well as `rust-toolchain.toml`.
    "rust-toolchain": ["ci.definingPaths", "review.classRules.policy"],
    // No cargo configuration exists. One could name a compiler wrapper.
    ".cargo/**": ["ci.definingPaths", "review.classRules.policy"],
    // No crate has a build script. It is code, so it is not policy.
    "**/build.rs": ["ci.definingPaths"],
    // rustfmt and clippy run with their defaults.
    "rustfmt.toml": ["ci.definingPaths", "review.classRules.policy"],
    ".rustfmt.toml": ["ci.definingPaths", "review.classRules.policy"],
    "clippy.toml": ["ci.definingPaths", "review.classRules.policy"],
    ".clippy.toml": ["ci.definingPaths", "review.classRules.policy"],
  };
  const lists = {
    "ci.definingPaths": CONFIG.ci.definingPaths,
    "review.classRules.code-adjacent": CONFIG.review.classRules["code-adjacent"],
    "review.classRules.policy": CONFIG.review.classRules.policy,
    "review.classRules.design-spec": CONFIG.review.classRules["design-spec"],
    "review.sourceGlobs": CONFIG.review.sourceGlobs,
    "fixGuard.replicatedRuleGlobs": CONFIG.fixGuard.replicatedRuleGlobs,
    "tests.cargo.testOnlyFileGlobs": CONFIG.tests.cargo.testOnlyFileGlobs,
    // A directory, as the key asks; what cargo builds from it is `<dir>/*.rs`.
    "tests.cargo.integrationTestDirs": CONFIG.tests.cargo.integrationTestDirs.map((dir) => `${dir}/*.rs`),
  };
  let checked = 0;
  const reservedSeen = [];
  for (const [key, entries] of Object.entries(lists)) {
    for (const entry of entries) {
      checked++;
      const matches = matching(entry).length > 0;
      const reserved = (RESERVED[entry] ?? []).includes(key);
      if (reserved) reservedSeen.push(`${key}: ${entry}`);
      assert.equal(
        matches,
        !reserved,
        reserved
          ? `${key}: ${entry} matches a tracked file now — take it off RESERVED and decide what the new file needs`
          : `${key}: ${entry} matches no tracked file. A directory is \`dir/**\`, any depth is \`**/name\`; ` +
            "if the entry is deliberate, add it to RESERVED with the reason",
      );
    }
  }
  // `review.diffExcludes` are git pathspecs, so git is asked.
  for (const spec of CONFIG.review.diffExcludes) {
    checked++;
    const hit = execFileSync("git", ["ls-files", "--", spec], { cwd: ROOT, encoding: "utf8" }).trim();
    assert.ok(hit !== "", `review.diffExcludes: ${spec} matches no tracked file`);
  }
  // Guards on the guard: the lists were walked, and RESERVED holds nothing stale.
  assert.ok(checked >= 40, `expected every configured entry, checked ${checked}`);
  assert.deepEqual(reservedSeen.sort(), Object.entries(RESERVED).flatMap(([e, keys]) => keys.map((k) => `${k}: ${e}`)).sort());
  // Every lane-configuration file that is CI-defining is reviewed as policy,
  // bar the two that are code (the generator and a build script).
  const code = new Set(["tools/protogen/**", "**/build.rs"]);
  for (const entry of CONFIG.ci.definingPaths.filter((e) => !code.has(e))) {
    assert.ok(CONFIG.review.classRules.policy.includes(entry), `${entry} defines CI but is not reviewed as policy`);
  }
});

// --- the toolchain action -----------------------------------------------------------
//
// The workflows cannot check what the action does: it is a different file in
// every repository. These pin the contract in scripts/agent/README.md ("The
// toolchain action") for this one.

test("the toolchain action installs the Rust that rust-toolchain.toml and ci.yml name", () => {
  // The price of not reading the branch's toolchain file is a second copy of
  // the version, and a third in ci.yml. A copy that goes stale shows up as
  // the fixer and CI disagreeing about formatting or lints.
  const pinned = /^\s+PINNED_TOOLCHAIN: "(\d+\.\d+\.\d+)"$/m.exec(ACTION_CODE)?.[1];
  assert.ok(pinned, "the toolchain action must pin the toolchain as a literal `PINNED_TOOLCHAIN`");
  assert.equal(pinned, RUST_VERSION, "the toolchain action's Rust is not rust-toolchain.toml's");
  const inCi = [...codeOf(CI).matchAll(/^\s+toolchain: (\S+)$/gm)].map((m) => m[1]);
  assert.deepEqual(inCi, [RUST_VERSION, RUST_VERSION], "ci.yml's two jobs must install rust-toolchain.toml's version");
  // The components `make verify` needs, as the toolchain file lists them.
  assert.match(read("rust-toolchain.toml"), /^components = \["rustfmt", "clippy"\]$/m);
  assert.match(ACTION_CODE, /rustup toolchain install "\$PINNED_TOOLCHAIN" \\\n\s+--profile minimal --component rustfmt --component clippy --no-self-update\n/);
  // And it is governed like the workflows that call it.
  assert.equal(checks.definesCi(".github/actions/agent-toolchain/action.yml"), true);
  assert.equal(panel.classifyFile(".github/actions/agent-toolchain/action.yml"), "policy");
});

test("the toolchain action runs nothing the branch wrote, uses no other action and saves no cache", () => {
  // COMPILING IS EXECUTING. Every cargo subcommand that builds runs the build
  // scripts and procedural macros of the crates the branch's lockfile names,
  // and any cargo command started in the workspace reads the branch's
  // `.cargo/config.toml`. So the action never invokes cargo, under any name.
  const cargoCall = /(^|[\s;&|(`"'])cargo(-[a-z-]+)?(\s|$)/m;
  assert.ok(cargoCall.test("  run: cargo fetch") && cargoCall.test('x="$(cargo --version)"') && cargoCall.test("a && cargo-clippy"), "the pattern must find a cargo call");
  assert.ok(!cargoCall.test('export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"'), "…and not the directory rustup lives in");
  assert.ok(!cargoCall.test(ACTION_CODE), "the toolchain action invokes cargo");
  assert.ok(!/\bmake\b/.test(ACTION_CODE), "the toolchain action runs make, which is the branch's Makefile");

  // One step, a shell script. No `uses:` at all: any other action would run
  // with this job's tokens, and `ci.yml`'s is a moving ref.
  assert.ok(!/\buses:/.test(ACTION_CODE), "the toolchain action uses another action");
  assert.equal([...ACTION_CODE.matchAll(/^ {4}- /gm)].length, 1, "the toolchain action has more than its one step");
  assert.match(ACTION_CODE, /^ {2}using: composite$/m);
  // No cache, by any name: a post step would save what the agent left behind
  // under a key the agent controls, and ci.yml restores rust-cache on main.
  assert.ok(!/cache/i.test(ACTION_CODE), "the toolchain action mentions a cache");

  // It never reads the branch's toolchain file, manifests or cargo config.
  for (const name of ["rust-toolchain", "Cargo.toml", "Cargo.lock", "config.toml"]) {
    assert.ok(!ACTION_CODE.includes(name), `the toolchain action reads ${name}, which is the branch's`);
  }
  // It takes no input and declares no output: the callers pass and read none.
  assert.ok(!/^(inputs|outputs):/m.test(ACTION_CODE));
  assert.ok(!ACTION_CODE.includes("${{"), "the toolchain action interpolates an expression into its script");
});

test("the toolchain action cannot be redirected by the workspace, and neither can the steps after it", () => {
  const script = ACTION_CODE.slice(ACTION_CODE.indexOf("      run: |"));
  const at = (needle) => {
    const i = script.indexOf(needle);
    assert.ok(i >= 0, `the toolchain action no longer contains \`${needle}\``);
    return i;
  };
  assert.match(script, /^\s+set -euo pipefail$/m);
  // rustup looks for a toolchain file from the working directory upwards, so
  // it leaves the workspace before the first rustup or rustc command, and
  // refuses a temporary directory that is inside the workspace.
  const leave = at('cd -- "$RUNNER_TEMP"');
  const firstTool = script.search(/^\s+(?:if ! )?(?:command -v )?(rustup|rustc|"\$dir\/rustup-init")\b/m);
  assert.ok(firstTool > leave, "the toolchain action touches rustup before it has left the workspace");
  assert.ok(at("is inside the workspace") > leave && at("is inside the workspace") < firstTool);
  // …and never goes back: the only other `cd` is the subshell that resolves
  // the workspace's real path.
  assert.deepEqual([...script.matchAll(/\bcd [^\n;&)]*/g)].map((m) => m[0].trim()), ['cd -- "$GITHUB_WORKSPACE"', 'cd -- "$RUNNER_TEMP"']);
  assert.ok(at("unset RUSTUP_TOOLCHAIN") < firstTool);
  // Every rustup and rustc command that chooses a toolchain names the pin.
  for (const line of script.split("\n").map((l) => l.trim()).filter((l) => /\brustup (toolchain|default|component|run|show|which)\b|\brustc /.test(l))) {
    assert.ok(line.includes("$PINNED_TOOLCHAIN"), `this line lets rustup choose the toolchain: ${line}`);
  }
  assert.ok(!/rustup override/.test(script));
  // The rest of the job: rustup lets RUSTUP_TOOLCHAIN win over a toolchain
  // file and over a directory override, so the branch's file cannot choose
  // the compiler the agent's `make verify` runs.
  at('echo "RUSTUP_TOOLCHAIN=$PINNED_TOOLCHAIN" >> "$GITHUB_ENV"');
  // It ends by checking what it installed.
  assert.ok(at('"rustc $PINNED_TOOLCHAIN "*)') > at("rustup toolchain install"));

  // WHERE THE RUNNER HAS NO RUSTUP: one binary, from rust-lang.org, at a
  // pinned version, executed only after its checksum matched. Never a script
  // piped into a shell.
  assert.match(ACTION_CODE, /^\s+RUSTUP_INIT_VERSION: "\d+\.\d+\.\d+"$/m);
  for (const arch of ["X86_64", "AARCH64"]) assert.match(ACTION_CODE, new RegExp(`^\\s+RUSTUP_INIT_SHA256_${arch}: "[0-9a-f]{64}"$`, "m"));
  at('"https://static.rust-lang.org/rustup/archive/$RUSTUP_INIT_VERSION/$triple/rustup-init"');
  assert.equal([...script.matchAll(/https?:\/\//g)].length, 1, "the toolchain action fetches from somewhere else too");
  assert.ok(at("sha256sum -c -") < at('"$dir/rustup-init" -y'), "rustup-init must be checked before it is run");
  assert.ok(!/\|\s*(ba)?sh\b/.test(script), "the toolchain action pipes a download into a shell");
  assert.match(script, /"\$dir\/rustup-init" -y --no-modify-path --profile minimal --default-toolchain none\n/);
});

// --- the commands the prompts name --------------------------------------------------

test("make verify is exactly what ci.yml's check job runs", () => {
  // `fixer.verifyCommand` is the whole of what a fixer verifies with. A
  // hand-written subset would let a fixer push something CI then refuses, so
  // the check job calls the same targets and nothing else.
  assert.equal(CONFIG.fixer.verifyCommand, "make verify");
  const verify = makeTarget("verify");
  assert.deepEqual(verify.needs, ["fmt-check", "clippy", "test", "doc"]);
  assert.deepEqual(verify.recipe, []);
  assert.deepEqual(runLines(ciJob("check")), verify.needs.map((t) => `make ${t}`));
  // The four commands, as the prompts and the coverage note describe them.
  assert.deepEqual(makeTarget("fmt-check").recipe, ["cargo fmt --all --check"]);
  assert.deepEqual(makeTarget("clippy").recipe, ["cargo clippy --workspace --all-targets -- -D warnings"]);
  assert.deepEqual(makeTarget("test").recipe, ["cargo test --workspace"]);
  assert.deepEqual(makeTarget("doc").recipe, ['RUSTDOCFLAGS="-D warnings" cargo doc -p yorkie --no-deps']);
  for (const t of verify.needs) assert.deepEqual(makeTarget(t).needs, [], `make ${t} runs another target first`);
  // Nothing in the check job's environment changes what those commands do.
  assert.ok(!/RUSTFLAGS|RUSTDOCFLAGS|CARGO_(?!TERM_COLOR)/.test(codeOf(CI)), "ci.yml sets a cargo or rustc variable the Makefile does not");
  assert.match(CONFIG.fixer.verifyDescription, /`cargo fmt --all --check`.*clippy.*warnings denied.*`cargo test --workspace`.*rustdoc/);
});

test("every command, test and lane a prompt names exists here", () => {
  const said = Object.values(CONFIG.fixer).map(String).join("\n");
  // make targets.
  const targets = new Set([...said.matchAll(/`make ([a-z-]+)`/g)].map((m) => m[1]));
  assert.deepEqual([...targets], ["verify"]);
  for (const name of targets) makeTarget(name);
  // `cargo run -p <member>`: the generator.
  const packages = new Set([...said.matchAll(/`cargo run -p ([a-z-]+)`/g)].map((m) => m[1]));
  assert.deepEqual([...packages], ["protogen"]);
  assert.ok(MEMBERS.includes("tools/protogen"));
  assert.match(read("tools", "protogen", "Cargo.toml"), /^name = "protogen"$/m);
  // The freshness test the hint names.
  assert.ok(said.includes("`protogen_output_is_up_to_date`"));
  assert.match(read("tools", "protogen", "tests", "up_to_date.rs"), /^fn protogen_output_is_up_to_date\(\)/m);
  // The variable that switches the integration tests on: read by their
  // support module, set by ci.yml's integration job and by no other.
  assert.ok(said.includes("`YORKIE_TEST_RPC_ADDR`"));
  assert.match(read("crates", "yorkie", "tests", "integration_support", "mod.rs"), /std::env::var\("YORKIE_TEST_RPC_ADDR"\)/);
  assert.match(ciJob("integration"), /^ {6}YORKIE_TEST_RPC_ADDR: http:\/\/localhost:8080$/m);
  assert.ok(!codeOf(ciJob("check")).includes("YORKIE_TEST_RPC_ADDR"));
  assert.ok(matching("crates/yorkie/tests/integration_*.rs").length >= 20, "the integration targets the warning names moved");
  // The lanes, by the names GitHub shows.
  const lanes = new Set([...said.matchAll(/`([^`]+)` lane/g)].map((m) => m[1]));
  assert.deepEqual([...lanes].sort(), ["fmt, clippy, unit tests", "integration tests"]);
  assert.match(ciJob("check"), /^ {4}name: fmt, clippy, unit tests$/m);
  assert.match(ciJob("integration"), /^ {4}name: integration tests$/m);
  // The generated file and its inputs.
  for (const m of said.matchAll(/`(crates\/[^`*<]+)`/g)) {
    assert.ok(isTracked(m[1]) || TRACKED.some((f) => f.startsWith(m[1])), `a prompt names ${m[1]}, which is not in the repository`);
  }
});

test("the fixer is told this repository's Rust conventions, and nothing a runner does not enforce", () => {
  // The reproducer convention is the one the cargo rules record.
  assert.match(CONFIG.fixer.keepReproducerAs, /`#\[ignore = "still reproduces: <finding>"\]` directly under its test attribute/);
  assert.equal(CONFIG.fixer.disabledTestForms, "switches off with `#[ignore]`, `#[should_panic]` or a `#[cfg(...)]` gate");
  // The commit convention is the one CLAUDE.md states, and no hook checks it:
  // this repository has none, and the harness installs only its scope hook.
  assert.match(CONFIG.fixer.commitNote, /a subject of 70 characters or less that starts with a verb, a blank line, a body wrapped at 80\./);
  const claude = read("CLAUDE.md");
  assert.match(claude, /starts with a verb and says what changed, in 70 characters or\s+less/);
  assert.match(claude, /body wrapped at 80 columns/);
  assert.ok(!TRACKED.some((f) => f.startsWith(".githooks/") || f === "scripts/setup.sh"), "a hook layer exists now: does the commit note still say enough?");
  // The Rust scope rule, in the words the scope hook uses when it refuses.
  assert.match(CONFIG.fixer.commitNote, /inside a top-level `#\[cfg\(test\)\] mod` of the file it tests/);
  assert.match(CONFIG.fixer.commitNote, /a new `\*_tests\.rs` file declared by a `#\[cfg\(test\)\] mod <name>;` line in its parent module/);
  assert.match(CONFIG.fixer.commitNote, /the scope hook refuses any other change to a Rust file outside the pull request's diff\.$/);
  // No licence header and no tracking line: there is no convention to state.
  assert.equal(CONFIG.fixer.licenceScope, "");
  assert.equal(CONFIG.fixer.todoTrackingLine, "");
  assert.ok(!TRACKED.includes("LICENSE") && !/^\/\/ Copyright|Licensed under/m.test(read("crates", "yorkie-core", "src", "lib.rs")), "a licence header exists now: say so in fixer.licenceScope");
  // The issue-to-PR step may use the prompt's own placeholders, and only those.
  const placeholders = [...CONFIG.fixer.implementVerifyStep.matchAll(/__[A-Z_]+__/g)].map((m) => m[0]);
  assert.deepEqual(placeholders, ["__PR_FILE__"]);
});

// --- the cargo test rules -----------------------------------------------------------

/** Every `mod name;` line of a Rust file, with whether it sits in a top-level `#[cfg(test)]` region and where it points. */
function moduleDeclarations(file) {
  const src = read(file);
  const parsed = cargoRules.testRegions(src);
  assert.ok(parsed, `${file} does not lex`);
  const inTest = (line) => parsed.regions.find((r) => r.startLine <= line && line <= r.endLine);
  const dir = path.posix.dirname(file);
  const stem = path.posix.basename(file, ".rs");
  // Where this file's child modules live: beside a `mod.rs`, `lib.rs` or
  // `main.rs`, and under `<stem>/` for any other file.
  const base = ["mod", "lib", "main"].includes(stem) ? dir : `${dir}/${stem}`;
  const lines = src.split("\n");
  const out = [];
  lines.forEach((text, i) => {
    const m = /^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;/.exec(text);
    if (!m) return;
    const region = inTest(i + 1);
    const attr = /^\s*#\[path\s*=\s*"([^"]+)"\]/.exec(lines[i - 1] ?? "");
    // `#[path]` on a top-level declaration is relative to the file's
    // directory. A declaration nested in an inline module lives one
    // directory further down, named after that module.
    const nested = region && region.kind === "inline" ? `${base}/${region.name}` : base;
    const target = attr ? path.posix.join(dir, attr[1]) : [`${nested}/${m[1]}.rs`, `${nested}/${m[1]}/mod.rs`].find(isTracked);
    out.push({ file, line: i + 1, name: m[1], test: Boolean(region), target: target ?? null });
  });
  return out;
}

test("the cargo rules classify this repository's four test layouts as the rule set intends", () => {
  // `vitest` stays beside `cargo` for the harness's own tests, which every
  // repository carries. `go` is not named: the one Go file here
  // (tools/interop) is a source copied into the server repository to be run
  // there, and nothing in this repository runs it.
  assert.deepEqual([...CONFIG.tests.rules], ["cargo", "vitest"]);
  assert.equal(CONFIG.tests.go, undefined);
  // Cargo builds integration targets from each package's `tests/`.
  assert.deepEqual([...CONFIG.tests.cargo.integrationTestDirs], ["crates/*/tests", "tools/*/tests"]);
  const packagesWithTests = MEMBERS.filter((m) => TRACKED.some((f) => f.startsWith(`${m}/tests/`)));
  assert.ok(packagesWithTests.length >= 4);
  for (const m of packagesWithTests) assert.match(m, /^(crates|tools)\/[^/]+$/, `${m} keeps tests where integrationTestDirs does not look`);

  const { isTestFile, isRunnableTest, isCargoUnitTestByName } = testRemovals;
  // 1. A source file with a `mod tests` at the bottom is NOT a test file: its
  //    tests are judged by region, not by path.
  for (const f of ["crates/yorkie-core/src/crdt/tree.rs", "crates/yorkie-core/src/document.rs", "crates/yorkie-client/src/client.rs"]) {
    assert.ok(isTracked(f), f);
    assert.match(read(f), /^#\[cfg\(test\)\]\nmod tests \{$/m, `${f} no longer ends in an inline test module`);
    assert.equal(isTestFile(f), false, f);
    assert.equal(isRunnableTest(f), false, f);
  }
  // 2. A `*_tests.rs` / `tests.rs` file is a test file by its name, and in a
  //    fix round's scope once its parent's `#[cfg(test)] mod` line is read.
  for (const f of ["crates/yorkie-core/src/crdt/tree_undo_tests.rs", "crates/yorkie-core/src/history/tests.rs", "crates/yorkie-core/src/document/gc_tests.rs"]) {
    assert.ok(isTracked(f), f);
    assert.equal(isTestFile(f), true, f);
    assert.equal(isCargoUnitTestByName(f), true, f);
    assert.equal(isRunnableTest(f), false, f);
    assert.equal(fixGuard.inScope(f, new Set()), false, `${f} must not be in scope by its name alone`);
    assert.equal(fixGuard.rustDeclared(f, (p) => (isTracked(p) ? read(p) : null)), true, f);
  }
  // 3. An integration target is a test file cargo runs by name…
  for (const f of ["crates/yorkie/tests/integration_tree.rs", "crates/yorkie-core/tests/document.rs", "tools/protogen/tests/up_to_date.rs"]) {
    assert.ok(isTracked(f), f);
    assert.equal(isTestFile(f), true, f);
    assert.equal(isRunnableTest(f), true, f);
    assert.equal(fixGuard.inScope(f, new Set()), true, f);
  }
  //    …and its helper module is a test file that is not a target.
  const helper = "crates/yorkie/tests/integration_support/mod.rs";
  assert.ok(isTracked(helper));
  assert.equal(isTestFile(helper), true);
  assert.equal(isRunnableTest(helper), false);
  // 4. The files `testOnlyFileGlobs` lists are test files by configuration.
  for (const f of CONFIG.tests.cargo.testOnlyFileGlobs.flatMap(matching)) {
    assert.equal(isTestFile(f), true, f);
    assert.equal(fixGuard.inScope(f, new Set()), true, f);
  }
  // What is never a test, whatever it is called or wherever it sits.
  for (const f of ["crates/yorkie/examples/quickstart.rs", "tools/protogen/src/main.rs", "Cargo.toml", "crates/yorkie-core/Cargo.toml", "Cargo.lock", "crates/yorkie-core/build.rs", "crates/yorkie/tests/build.rs", ".cargo/config.toml"]) {
    assert.equal(isTestFile(f), false, f);
    assert.equal(fixGuard.inScope(f, new Set()), false, f);
  }
  // The interop sources: the TypeScript ones are named as tests and count as
  // such under the `vitest` rules; the Go one does not, with no `go` rules.
  assert.equal(isTestFile("tools/interop/text_undo_fixtures_test.ts"), true);
  assert.equal(isTestFile("tools/interop/text_undo_fixtures_test.go"), false);

  // A case, a switched-off test and a removed test module, as the record counts them.
  const patch = ["@@ -1,6 +1,6 @@", " mod tests {", "-    #[test]", "-    fn a() {}", "+    #[tokio::test]", "+    #[ignore = \"still reproduces: x\"]", "+    async fn b() {}", " }"].join("\n");
  const counted = testRemovals.countCases(patch, { file: "crates/yorkie-core/src/crdt/tree.rs", status: "modified" });
  assert.deepEqual(counted, { removed: 1, added: 1, suitesOff: 1 });
});

test("testOnlyFileGlobs lists exactly the test-only files the declaration check cannot follow", () => {
  // The scope check admits a `*_tests.rs` file when a parent module declares
  // it `#[cfg(test)] mod name;`. It does not follow three layouts this
  // repository uses, so those files are listed — and listing a file puts it
  // in a fix round's scope, so each must really be compiled only under
  // `#[cfg(test)]`:
  //   - `document/adv_sync.rs`: declared `#[cfg(test)] mod adv_sync;`, but
  //     its name does not say "test";
  //   - `client/tests/*.rs`: the files of a directory that is itself a test
  //     module (`#[cfg(test)] mod tests { mod disable_gc; … }` in client.rs);
  //   - `reissue_tests.rs` and `wire_text_link_tests.rs`: reached through
  //     `#[path = "…"]` under another module name.
  const sources = TRACKED.filter((f) => /^(crates|tools)\/[^/]+\/src\/.*\.rs$/.test(f));
  assert.ok(sources.length >= 80, `expected the workspace's source files, found ${sources.length}`);
  const declarations = sources.flatMap(moduleDeclarations);
  assert.ok(declarations.length >= 60, `expected the workspace's module declarations, found ${declarations.length}`);
  for (const d of declarations) assert.ok(d.target && isTracked(d.target), `${d.file}:${d.line} declares \`mod ${d.name};\` and no tracked file was found for it`);

  const underTest = new Set(declarations.filter((d) => d.test).map((d) => d.target));
  const inProduct = new Set(declarations.filter((d) => !d.test).map((d) => d.target));
  assert.ok(underTest.size >= 20, `expected the test modules in files of their own, found ${underTest.size}`);
  // No file is compiled both ways.
  assert.deepEqual([...underTest].filter((f) => inProduct.has(f)), []);

  // What the declaration check follows by itself: a file named `tests.rs` or
  // `*_tests.rs` whose parent module (`mod.rs` in its directory, `<dir>.rs`
  // beside the directory, or `lib.rs` / `main.rs` for a file directly in
  // `src/`) declares it `#[cfg(test)] mod <its name>;` with no `#[path]`.
  // Written out here because the harness's own check answers "no" for any
  // file the configuration already lists, which would let a needless entry
  // justify itself.
  const NAMED_AS_TEST = /(^|\/)src\/(.*\/)?(tests?|[^/]*_tests?)\.rs$/;
  const parents = (f) => {
    const dir = path.posix.dirname(f);
    return path.posix.basename(dir) === "src" ? [`${dir}/lib.rs`, `${dir}/main.rs`, `${dir}/mod.rs`] : [`${dir}/mod.rs`, `${dir}.rs`];
  };
  const followed = (f) =>
    NAMED_AS_TEST.test(f) && parents(f).some((p) => isTracked(p) && cargoRules.declaresTestModule(read(p), path.posix.basename(f, ".rs")));
  // …and so what has to be listed: reached only under `#[cfg(test)]`, and not followed.
  const mustList = [...underTest].filter((f) => !followed(f)).sort();
  const listed = [...new Set(CONFIG.tests.cargo.testOnlyFileGlobs.flatMap(matching))].sort();
  assert.deepEqual(listed, mustList);
  assert.deepEqual(listed, [
    "crates/yorkie-client/src/client/tests/disable_gc.rs",
    "crates/yorkie-client/src/client/tests/reissue.rs",
    "crates/yorkie-core/src/document/adv_sync.rs",
    "crates/yorkie-core/src/reissue_tests.rs",
    "crates/yorkie-core/src/wire_text_link_tests.rs",
  ]);
  // The harness agrees about every file that is not listed: it admits the
  // followed ones to a fix round's scope and no other source file.
  const harnessFollows = (f) => fixGuard.rustDeclared(f, (p) => (isTracked(p) ? read(p) : null));
  for (const f of sources.filter((s) => !listed.includes(s))) assert.equal(harnessFollows(f), followed(f), f);
  assert.ok([...underTest].filter(followed).length >= 15);
  // Every file NAMED as a unit test is one: declared under `#[cfg(test)]`, by
  // the check or by the list. A `*_tests.rs` that a plain `mod` reaches would
  // be product code the removal record takes for tests.
  const named = sources.filter((f) => NAMED_AS_TEST.test(f));
  assert.ok(named.length >= 20);
  for (const f of named) assert.ok(underTest.has(f), `${f} is named as a test module and no \`#[cfg(test)] mod\` line reaches it`);
});

// --- the fix guard ------------------------------------------------------------------

test("the fix guard's replicated rules are the CRDT and clock modules, and both other homes silence it", () => {
  // `crdt/` and `time/` hold the rules every replica must apply identically:
  // the same pair the JS SDK lists (`document/crdt/*.ts`, `document/time/*.ts`)
  // and the directory the server lists (`pkg/document/crdt/*.go`).
  assert.deepEqual([...CONFIG.fixGuard.replicatedRuleGlobs], ["crates/yorkie-core/src/crdt/*.rs", "crates/yorkie-core/src/time/*.rs"]);
  for (const f of ["crates/yorkie-core/src/crdt/tree.rs", "crates/yorkie-core/src/crdt/rga_tree_split.rs", "crates/yorkie-core/src/time/ticket.rs"]) {
    assert.ok(isTracked(f), f);
    assert.equal(fixGuard.crdtRuleChanges([{ filename: f }]).length, 1, f);
  }
  // A test file in those directories is not a rule, and neither is the code around them.
  for (const f of ["crates/yorkie-core/src/crdt/tree_undo_tests.rs", "crates/yorkie-core/src/document.rs", "crates/yorkie-core/tests/time.rs", "docs/crdt-parity/tree.md"]) {
    assert.ok(isTracked(f), f);
    assert.equal(fixGuard.crdtRuleChanges([{ filename: f }]).length, 0, f);
  }
  // This SDK ports from the JS SDK and must converge with the Go server, so a
  // pull request that names either has said where the rule's other home is.
  assert.deepEqual([...CONFIG.fixGuard.counterpartRepos], ["yorkie-team/yorkie-js-sdk", "yorkie-team/yorkie"]);
  assert.ok(!CONFIG.fixGuard.counterpartRepos.includes("yorkie-team/yorkie-rust-sdk"));
  const files = [{ filename: "crates/yorkie-core/src/crdt/tree.rs" }];
  for (const body of ["Ports yorkie-js-sdk#1482.", "As in https://github.com/yorkie-team/yorkie-js-sdk/commit/84068f70", "Go side: yorkie-team/yorkie#2079", "see yorkie#2079"]) {
    assert.deepEqual(fixGuard.lintRound({ files, prBody: body }).crdt, [], body);
  }
  for (const body of ["", "Fixes #12", "Fixes yorkie-team/yorkie-rust-sdk#12", "the yorkie server does the same"]) {
    assert.equal(fixGuard.lintRound({ files, prBody: body }).crdt.length, 1, body);
  }
  assert.match(CONFIG.fixGuard.replicatedRuleNotice, /^\*\*Replicated CRDT rule changed in Rust only \(warning\)\.\*\*/);
  assert.match(CONFIG.fixGuard.replicatedRuleNotice, /yorkie-js-sdk or yorkie change/);
});

// --- file classes, lens scoping and the reviewed diff -------------------------------

test("classifyFile: lane configuration is policy, fixtures are code-adjacent, the porting contract is design-spec", () => {
  // What cargo, rustfmt, clippy and make run is the difference between a
  // mechanical guarantee and a claim.
  for (const f of ["Makefile", "Cargo.toml", "crates/yorkie-core/Cargo.toml", "tools/protogen/Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml", "clippy.toml"]) {
    assert.equal(panel.classifyFile(f), "policy", f);
  }
  // The harness's own dependencies: what the token-bearing jobs install.
  assert.equal(panel.classifyFile("scripts/agent/package-lock.json"), "policy");
  // This directory governs the agents: policy to the reviewers, CI-defining to the gate.
  for (const file of ["config.json", "coverage-note.md", "repo.snapshot.json", "repo.test.mjs", "upstream.json"]) {
    assert.equal(panel.classifyFile(`.github/agent-harness/${file}`), "policy", file);
    assert.equal(checks.definesCi(`.github/agent-harness/${file}`), true, file);
  }
  assert.equal(panel.classifyFile("CLAUDE.md"), "policy");
  assert.equal(panel.classifyFile("CONTRIBUTING.md"), "policy");
  assert.equal(panel.classifyFile(".github/PULL_REQUEST_TEMPLATE.md"), "policy");

  // Data the interop tests decode is read as code — its README included.
  assert.equal(panel.classifyFile("crates/yorkie-protocol/tests/fixtures/text_undo/ledger_snapshot_pack.hex"), "code-adjacent");
  assert.equal(panel.classifyFile("crates/yorkie-protocol/tests/fixtures/text_undo/README.md"), "code-adjacent");

  // THE DESIGN CONTRACT LIVES OUTSIDE docs/design HERE. The porting rules, the
  // dependency boundaries, the parity checklists, the known-gaps list and the
  // milestone plans are what design-fit measures a port against; as prose no
  // code lens would read them and design-fit would not run on them.
  for (const f of ["docs/porting-from-js-sdk.md", "docs/dependency-boundaries.md", "docs/current-porting-gaps.md", "docs/crdt-parity/tree.md", "docs/crdt-parity/README.md", "docs/crdt-parity/upstream-skipped-tests.md", "docs/plans/2026-11-09-tree-undo.md", "docs/design/TEMPLATE.md"]) {
    assert.ok(isTracked(f), f);
    assert.equal(panel.classifyFile(f), "design-spec", f);
  }
  // Every document under docs/ that is not a task file or an index is in it:
  // a new contract document in a new place fails here.
  const prose = TRACKED.filter((f) => f.startsWith("docs/") && panel.classifyFile(f) === "prose");
  assert.deepEqual(prose.filter((f) => !f.startsWith("docs/tasks/")), ["docs/README.md"]);
  assert.equal(panel.classifyFile("docs/tasks/active/20261114-agent-harness-todo.md"), "prose");
  assert.equal(panel.classifyFile("README.md"), "prose");

  // Rust is code, test source and generated source included; so are the
  // protocol definitions and a build script.
  for (const f of [
    "crates/yorkie-core/src/crdt/tree.rs",
    "crates/yorkie-core/src/crdt/tree_undo_tests.rs",
    "crates/yorkie/tests/integration_tree.rs",
    "crates/yorkie/examples/quickstart.rs",
    "crates/yorkie-protocol/src/generated/yorkie.v1.rs",
    "crates/yorkie-protocol/proto/yorkie/v1/resources.proto",
    "tools/interop/text_undo_fixtures_test.go",
    "crates/yorkie-core/build.rs",
  ]) {
    assert.equal(panel.classifyFile(f), "code", f);
  }
});

test("review.sourceGlobs covers every workspace member, and the path-scoped lenses run on them", () => {
  const lenses = panel.loadLenses(path.join(AGENT, "lenses"));
  const lensManifest = JSON.parse(readFileSync(path.join(AGENT, "lenses", "lenses.json"), "utf8"));
  const lensOf = (id) => lenses.find((l) => l.id === id);
  assert.ok(lenses.length >= 6, `expected the whole panel, loaded ${lenses.length} lenses`);

  const source = CONFIG.review.sourceGlobs.map(compileGlob);
  for (const m of MEMBERS) assert.ok(source.some((re) => re.test(`${m}/src/lib.rs`)), `${m} is a workspace member review.sourceGlobs does not cover`);
  // …and every tracked Rust file, wherever it is. (The harness package
  // carries Rust fixtures of its own; `scripts/**` is in the lenses' scope.)
  for (const f of TRACKED.filter((t) => t.endsWith(".rs") && !t.startsWith("scripts/agent/"))) assert.ok(source.some((re) => re.test(f)), `${f} is Rust that review.sourceGlobs does not cover`);

  // A code PR runs every path-scoped lens; a task-file-only PR runs none of them.
  const code = ["crates/yorkie-core/src/crdt/tree.rs"];
  for (const id of ["correctness", "security", "design-fit", "test-adequacy", "blast-radius"]) assert.equal(panel.lensApplies(lensOf(id), code), true, id);
  assert.equal(panel.lensApplies(lensOf("docs"), code), false);
  const reviewing = reviewScope.reviewingLensIds(lensManifest, code);
  for (const id of ["test-adequacy", "blast-radius"]) assert.ok(reviewing.includes(id), `${id} must review a code change`);
  const tasksOnly = reviewScope.reviewingLensIds(lensManifest, ["docs/tasks/active/20261114-x-todo.md"]);
  for (const id of ["test-adequacy", "blast-radius", "design-fit"]) assert.ok(!tasksOnly.includes(id), id);
  // A change to the porting contract alone is read by design-fit.
  assert.equal(panel.lensApplies(lensOf("design-fit"), ["docs/current-porting-gaps.md"]), true);
  assert.equal(panel.lensApplies(lensOf("design-fit"), ["docs/plans/2026-11-09-tree-undo.md"]), true);
});

test("the reviewed diff leaves out the generated protobuf code, and only that", () => {
  // 70 KB of prost output that no reviewer reads line by line. It is safe to
  // leave out because a lane proves it: `protogen_output_is_up_to_date`
  // regenerates it and fails when the checked-in file differs.
  assert.deepEqual([...CONFIG.review.diffExcludes], ["crates/yorkie-protocol/src/generated/*"]);
  const excluded = CONFIG.review.diffExcludes.flatMap((spec) =>
    execFileSync("git", ["ls-files", "--", spec], { cwd: ROOT, encoding: "utf8" }).split("\n").filter(Boolean));
  assert.deepEqual(excluded, ["crates/yorkie-protocol/src/generated/yorkie.v1.rs"]);
  assert.equal(workflowValues.values(["diff_pathspecs"])[0][1], ".\n:(exclude)crates/yorkie-protocol/src/generated/*");
  // The generator writes that file…
  const generator = read("tools", "protogen", "src", "lib.rs");
  assert.match(generator, /crates\/yorkie-protocol\/src\/generated\/yorkie\.v1\.rs/);
  // …the freshness test compares against it, in the lane every PR runs…
  const fresh = read("tools", "protogen", "tests", "up_to_date.rs");
  assert.match(fresh, /^#\[test\]\nfn protogen_output_is_up_to_date\(\)/m);
  assert.match(fresh, /protogen::generated_file\(\)/);
  assert.match(fresh, /protogen::generate\(/);
  assert.ok(!/#\[ignore/.test(fresh), "the freshness test is ignored, so nothing proves the excluded file");
  assert.ok(MEMBERS.includes("tools/protogen"), "`cargo test --workspace` no longer reaches the freshness test");
  // …and the fixtures stay IN the diff: they are bytes written by the other
  // SDKs that no lane can regenerate here.
  for (const f of matching("crates/yorkie-protocol/tests/fixtures/**")) assert.ok(!excluded.includes(f), f);
});

// --- the mechanical-coverage note ---------------------------------------------------

test("the coverage note names every CI lane, and claims only what this repository runs", () => {
  // The failure mode is SILENT: tell a lens something is covered when it is
  // not and that finding class stops being reported, with nothing in the
  // output to show for it. Each assertion pins a fact read off this
  // repository, so a change to a lane breaks this test instead of quietly
  // making the note a lie.
  const N = panel.MECHANICAL_COVERAGE_NOTE;
  assert.equal(N, read(".github", "agent-harness", "coverage-note.md").replace(/\n$/, ""));
  assert.equal(N.split("\n")[0], "## What the mechanical lanes cover on this branch");
  assert.ok(N.indexOf("ENFORCED") >= 0 && N.indexOf("NOT ENFORCED") > N.indexOf("ENFORCED"), "the note needs both halves, in order");
  // A note that says "don't report X" is exactly the shape a clamp hides in.
  for (const clamp of [/when unsure,?\s+downgrade/i, /mark it minor/i, /only with (?:a )?concrete/i, /ONLY for a\s+concrete/i, /severity ONLY/i]) {
    assert.ok(!clamp.test(N), `coverage-note.md re-introduces a certainty clamp: ${clamp}`);
  }
  const flat = (s) => s.replace(/\s+/g, " ");
  const enforced = flat(N.slice(N.indexOf("ENFORCED"), N.indexOf("NOT ENFORCED")));
  const notEnforced = flat(N.slice(N.indexOf("NOT ENFORCED")));

  // EVERY LANE, read from ci.yml and the Makefile rather than typed here: each
  // command the check job runs, through the make target it calls…
  const commands = runLines(ciJob("check")).flatMap((cmd) => makeTarget(cmd.replace(/^make /, "")).recipe);
  assert.equal(commands.length, 4);
  for (const command of commands) {
    const m = /^(?:([A-Z_]+)="([^"]+)" )?(cargo .+)$/.exec(command);
    assert.ok(m, command);
    assert.ok(enforced.includes(`\`${m[3]}\``), `the note does not name the lane \`${m[3]}\``);
    if (m[1]) assert.ok(enforced.includes(`\`${m[1]}=${m[2]}\``), `the note does not say \`${m[3]}\` runs with ${m[1]}=${m[2]}`);
  }
  // …and each command of the integration job, with what it runs against.
  const integration = ciJob("integration");
  const integrationRuns = runLines(integration);
  assert.deepEqual(integrationRuns, ["cargo test -p yorkie", "cargo run -p yorkie --example quickstart", "docker logs yorkie"]);
  assert.ok(enforced.includes("The `integration tests` lane: `cargo test -p yorkie` with `YORKIE_TEST_RPC_ADDR` set"));
  assert.ok(enforced.includes("a run of the `quickstart` example"));
  const image = /^ {6}YORKIE_IMAGE: (\S+)$/m.exec(integration)?.[1];
  assert.match(String(image), /^yorkieteam\/yorkie:\d+\.\d+\.\d+$/);
  assert.ok(enforced.includes(`against \`${image}\` with its in-memory database`), `the note does not name the server image ${image}`);
  assert.match(integration, /server --rpc-port 8080 --profiling-port 8081$/m, "the server is started with a database now; the note says in-memory");
  assert.ok(notEnforced.includes(`any server but \`${image}\``));
  // Nothing else in ci.yml is a lane: two jobs, on one runner image.
  assert.deepEqual([...CI.matchAll(/^ {2}([a-z_-]+):\n {4}name: /gm)].map((m) => m[1]), ["check", "integration"]);
  assert.deepEqual([...new Set([...CI.matchAll(/^ {4}runs-on: (\S+)$/gm)].map((m) => m[1]))], ["ubuntu-latest"]);

  // The versions and defaults the note states.
  assert.ok(enforced.includes(`on Rust ${RUST_VERSION}`) && notEnforced.includes(`Any Rust but ${RUST_VERSION}`), "the note's Rust version is not rust-toolchain.toml's");
  assert.ok(enforced.includes("there is no rustfmt.toml"));
  assert.ok(!TRACKED.some((f) => /(^|\/)\.?(rustfmt|clippy)\.toml$/.test(f)), "a rustfmt or clippy configuration exists now; the note says defaults");
  assert.ok(notEnforced.includes("any feature set but the default"));
  assert.ok(!/--(all-)?features|--no-default-features/.test(MAKEFILE + CI));
  // `unsafe` does not compile: every crate root forbids it.
  assert.ok(enforced.includes("Every crate root declares `#![forbid(unsafe_code)]`"));
  const roots = MEMBERS.flatMap((m) => ["lib.rs", "main.rs"].map((f) => `${m}/src/${f}`)).filter(isTracked);
  assert.ok(roots.length >= MEMBERS.length);
  for (const root of roots) assert.match(read(root), /^#!\[forbid\(unsafe_code\)\]$/m, `${root} does not forbid unsafe code; the note says every crate root does`);
  // "No path filter" is pinned by the trigger test above.
  assert.ok(enforced.includes("No path filter"));

  // THE GAP MOST LIKELY TO BE MISSED: the integration tests pass without
  // running. Each returns early when the variable is unset, and the
  // workspace lane runs them that way.
  assert.ok(notEnforced.includes("returns early, and passes, when `YORKIE_TEST_RPC_ADDR` is unset"));
  assert.match(read("crates", "yorkie", "tests", "integration_support", "mod.rs"), /std::env::var\("YORKIE_TEST_RPC_ADDR"\)[\s\S]{0,200}eprintln!\("skipping: /);
  // Ignored tests exist and no lane runs them.
  assert.ok(notEnforced.includes("Any test marked `#[ignore]`. No lane passes `--ignored`"));
  assert.ok(TRACKED.some((f) => f.endsWith(".rs") && /^\s*#\[ignore\b/m.test(read(f))), "no test is ignored any more; drop the bullet");
  assert.ok(!/--(include-)?ignored/.test(MAKEFILE + CI), "a lane runs the ignored tests now; move the bullet");
  // Rustdoc is built for the facade only.
  assert.ok(notEnforced.includes("Rustdoc for any crate but `yorkie`"));
  // None of these is configured. A lane that adds one moves its word up.
  for (const tool of ["cargo-audit", "cargo audit", "cargo-deny", "cargo deny", "miri", "llvm-cov", "tarpaulin", "semver-checks", "udeps", "machete"]) {
    assert.ok(!(MAKEFILE + CI).toLowerCase().includes(tool), `${tool} runs now; the note says it does not`);
  }

  // MECHANISMS, NEVER CATEGORIES: named tools and named lanes only.
  for (const category of [/\btype problems\b/i, /\btype issues\b/i, /\blint issues are\b/i, /\bstyle problems\b/i]) {
    assert.ok(!category.test(N), `the note must name a mechanism, not a category: ${category}`);
  }
  // The panel runs CONCURRENTLY with CI, so nothing here is proven at the
  // moment a lens reads it.
  assert.ok(!/already (?:caught|checked|passed|proven)/i.test(N), "the panel runs alongside CI — the note may not claim these have already passed");
  assert.match(N, /must pass before it can be promoted/);
});

// --- identity -----------------------------------------------------------------------

test("the bots the workflows believe are the ones the scripts believe", () => {
  const value = (name) => workflowValues.values([name])[0][1];
  assert.deepEqual(JSON.parse(value("bot_logins")), [...rounds.PAGE_AUTHOR_LOGINS]);
  assert.deepEqual(JSON.parse(value("bot_logins")), ["github-actions[bot]", "yorkie-team-agent[bot]"]);
  assert.equal(value("app_slug"), "yorkie-team-agent");
  assert.ok(CONFIG.repo.appLogins.includes(value("app_bot_login")));
  assert.equal(value("allowed_bots"), "yorkie-team-agent[bot],yorkie-team-agent");
});

test("the command-verbs reference is the server repository's document, by URL", () => {
  // The document is written in yorkie and not copied here, so a decline
  // message links to it. A path would be shown as a code span naming a file
  // this repository does not have.
  assert.equal(CONFIG.docs.commandVerbsRef, "https://github.com/yorkie-team/yorkie/blob/main/docs/design/agent-command-verbs.md");
  assert.equal(workflowValues.values(["docs_ref"])[0][1], CONFIG.docs.commandVerbsRef);
  assert.ok(!isTracked("docs/design/agent-command-verbs.md"));
});

test("no workflow here publishes on a release or inherits secrets on a push", () => {
  // `contents: write` covers commits AND releases, so the token a fix agent
  // pushes with can publish a release. This repository has no workflow that
  // answers one, and none that hands secrets to a push. The day it gets one,
  // that workflow must refuse the agent App as the author (as the server's
  // and the JS SDK's publishers do), and this test is where that is pinned.
  const others = readdirSync(WORKFLOWS).filter((f) => f.endsWith(".yml") && !/^agent-/.test(f));
  assert.deepEqual(others, ["ci.yml"]);
  for (const file of others) {
    const code = codeOf(readFileSync(path.join(WORKFLOWS, file), "utf8"));
    assert.ok(!/^\s+release:\s*$/m.test(code), `${file} runs on a release`);
    assert.ok(!/secrets\b/.test(code), `${file} reads a secret`);
  }
});

// --- what the workflows say, in this repository's words -----------------------------
//
// The `agent-*.yml` files carry no command, path or name of this repository's:
// they read each from the configuration through `workflow-values.mjs`.
// `repo.snapshot.json` records every such value and every prompt as rendered
// here, so a change to `config.json` — or a sync that changes a prompt's
// frame — shows its before and after in that file's diff.
//
// To record it again after reading the difference:
//   AGENT_HARNESS_RECORD=1 npm run test:repo

/** Collapse runs of whitespace: a prompt's line wrapping is not its text. */
const flat = (text) => text.replace(/\s+/g, " ").trim();

/** `text` with every configured-value expression replaced by this repository's value. */
function rendered(text) {
  const all = Object.fromEntries(workflowValues.values(Object.keys(workflowValues.VALUES)));
  return text.replace(/\$\{\{\s*(?:steps\.cfg|needs\.[\w-]+)\.outputs\.([a-z_]+)\s*\}\}/g, (whole, name) =>
    Object.hasOwn(all, name) ? all[name] : whole);
}

/** Every `prompt: |` block of a workflow, in order. */
function promptBlocks(text) {
  const lines = text.split("\n");
  const blocks = [];
  lines.forEach((line, i) => {
    const m = /^(\s+)prompt: \|\s*$/.exec(line);
    if (!m) return;
    const body = [];
    for (const next of lines.slice(i + 1)) {
      if (next.trim() !== "" && /^(\s*)/.exec(next)[1].length <= m[1].length) break;
      body.push(next);
    }
    blocks.push(body.join("\n"));
  });
  return blocks;
}

/** `NAME: ${{ steps.cfg.outputs.value }}` in a step's `env:`. */
const ENV_FROM_CONFIG = /^\s+([A-Z_]+): (\$\{\{ steps\.cfg\.outputs\.[a-z_]+ \}\})\s*$/gm;
/** A placeholder replaced by a QUOTED variable. */
const QUOTED_SUBSTITUTION = /^\s+TEMPLATE=\$\{TEMPLATE\/\/(__[A-Z_]+__)\/"\$([A-Z_]+)"\}\s*$/gm;

/**
 * The issue-to-PR prompt, which is a heredoc in a `run:` step. Configured
 * values reach it as environment variables substituted for `__PLACEHOLDER__`s,
 * so the same substitutions are made here.
 */
function heredocPrompt(text) {
  const start = text.indexOf("<<'PROMPT'\n");
  const end = text.indexOf("\n          PROMPT\n", start);
  assert.ok(start > 0 && end > start, "agent-implement.yml no longer builds its prompt in a `PROMPT` heredoc");
  let prompt = text.slice(start + "<<'PROMPT'\n".length, end);
  const stepAt = text.lastIndexOf("\n      - name: ", start);
  const stepEnd = text.indexOf("\n      - name: ", start);
  const step = text.slice(stepAt, stepEnd < 0 ? undefined : stepEnd);
  const env = new Map([...step.matchAll(ENV_FROM_CONFIG)].map((m) => [m[1], m[2]]));
  let substituted = 0;
  for (const m of step.matchAll(QUOTED_SUBSTITUTION)) {
    assert.ok(env.has(m[2]), `agent-implement.yml substitutes ${m[1]} with a variable its env does not set from the configuration`);
    prompt = prompt.replaceAll(m[1], env.get(m[2]));
    substituted++;
  }
  assert.ok(substituted >= 5, `expected the configured placeholders, found ${substituted}`);
  return [prompt];
}

function renderedPrompts() {
  const sites = [
    ["agent-fix.yml", promptBlocks],
    ["agent-review-panel.yml", promptBlocks],
    ["agent-iterate-ci.yml", promptBlocks],
    ["agent-review-reply.yml", promptBlocks],
    ["agent-summarize.yml", promptBlocks],
    ["agent-implement.yml", heredocPrompt],
  ];
  const got = {};
  for (const [file, extract] of sites) {
    const blocks = extract(readFileSync(path.join(WORKFLOWS, file), "utf8"));
    assert.ok(blocks.length > 0, `${file}: no prompt found`);
    blocks.forEach((block, i) => {
      const text = flat(rendered(block));
      assert.ok(!/(?:steps\.cfg|needs\.route)\.outputs\./.test(text), `${file}: a configured value was not rendered`);
      got[`${file}#${i}`] = text;
    });
  }
  return got;
}

const SNAPSHOT_PATH = path.join(HERE, "repo.snapshot.json");
const CURRENT = {
  WORKFLOW_VALUES: Object.fromEntries(workflowValues.values(Object.keys(workflowValues.VALUES))),
  WORKFLOW_PROMPTS: renderedPrompts(),
  CI_DEFINING_PATHS: [...checks.CI_DEFINING_PATHS],
  CLASS_RULES: JSON.parse(JSON.stringify(panel.CLASS_RULES)),
  ciGateName: checks.CI_GATE_NAME,
  handoffCiBullet: [...checks.HANDOFF_CI_SUMMARY],
};
// Never set in CI: `agent-scripts.yml` runs `npm run test:repo` with no such variable.
if (process.env.AGENT_HARNESS_RECORD === "1") writeFileSync(SNAPSHOT_PATH, `${JSON.stringify(CURRENT, null, 2)}\n`);
const SNAPSHOT = JSON.parse(readFileSync(SNAPSHOT_PATH, "utf8"));

test("snapshot: every value the workflows read from the configuration", () => {
  assert.deepEqual(CURRENT.WORKFLOW_VALUES, SNAPSHOT.WORKFLOW_VALUES);
});

test("snapshot: the prompts the agents are given, as rendered for this repository", () => {
  assert.ok(Object.keys(CURRENT.WORKFLOW_PROMPTS).length >= 6, "expected a prompt from each agent workflow");
  assert.deepEqual(CURRENT.WORKFLOW_PROMPTS, SNAPSHOT.WORKFLOW_PROMPTS);
});

test("snapshot: the CI-defining paths, the file-class rules and what the hand-off says CI covered", () => {
  assert.deepEqual(CURRENT.CI_DEFINING_PATHS, SNAPSHOT.CI_DEFINING_PATHS);
  assert.deepEqual(CURRENT.CLASS_RULES, SNAPSHOT.CLASS_RULES);
  assert.equal(CURRENT.ciGateName, SNAPSHOT.ciGateName);
  assert.deepEqual(CURRENT.handoffCiBullet, SNAPSHOT.handoffCiBullet);
  assert.equal(process.env.AGENT_HARNESS_RECORD, undefined, "this run RECORDED the snapshot; run again without AGENT_HARNESS_RECORD");
});
