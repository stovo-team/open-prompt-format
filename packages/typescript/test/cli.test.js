import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { runCli } from "../dist/cli.js";

const execFileAsync = promisify(execFile);
const packageDir = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(packageDir, "../../..");
const cliPath = path.join(packageDir, "../dist/cli.js");

test("CLI validate and validate-collection succeed on examples", async () => {
  const single = await runCli(["validate", path.join(root, "examples/support.reply.md")]);
  assert.equal(single.exitCode, 0);
  assert.match(single.stdout, /^valid: support\.reply \(sha256:[0-9a-f]{64}\)$/);

  const collection = await runCli(["validate-collection", path.join(root, "examples")]);
  assert.equal(collection.exitCode, 0);
  assert.match(collection.stdout, /^valid: \d+ prompt\(s\)$/);
});

test("CLI render outputs role/content JSON and compat outputs JSON report", async () => {
  const rendered = await runCli([
    "render",
    path.join(root, "examples/support.reply.md"),
    "--input",
    "customer_message=Where is my order?",
  ]);
  const messages = JSON.parse(rendered.stdout);
  assert.equal(messages[0].role, "system");
  assert.equal(messages[1].role, "user");
  assert.equal(messages[1].content, "Where is my order?");

  const compat = await runCli(["compat", path.join(root, "examples/support.reply.md"), "--format", "json"]);
  const report = JSON.parse(compat.stdout);
  assert.equal(report.source_kind, "opf/0.2");
  assert.equal(report.target_kind, "opf-core");
  assert.deepEqual(report.findings, []);
});

test("CLI subprocess verify-all and error exit code work", async () => {
  const { stdout } = await execFileAsync(process.execPath, [
    cliPath,
    "verify-all",
    "--registry",
    path.join(root, "opf.yaml"),
  ]);
  assert.deepEqual(JSON.parse(stdout), []);

  await assert.rejects(
    execFileAsync(process.execPath, [cliPath, "unknown-cmd"]),
    (err) => err.code === 1 && /unknown command/.test(err.stderr),
  );
});
