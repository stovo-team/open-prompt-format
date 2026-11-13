#!/usr/bin/env node
import { OPFError, Registry, load, loadCollection } from "./index.js";

function usage(): string {
  return [
    "Usage: opf <command> [options]",
    "",
    "Commands:",
    "  validate <file>                       Validate an OPF prompt file",
    "  validate-collection <dir>             Validate all OPF prompts in a directory",
    "  compat <file> [--format json|text]    Report OPF extension compatibility",
    "  render <target> [options]             Render a prompt or registry release to JSON",
    "    --input k=v                         Input variable (repeatable)",
    "    --registry <path>                   Registry file (default: opf.yaml when --version/--channel is set)",
    "    --version <semver>                  Release version to render",
    "    --channel <name>                    Release channel to render",
    "    --receipt                           Include render receipt in JSON output",
    "  verify <id> [options]                 Verify a released prompt or channel",
    "    --registry <path>                   Registry file (default: opf.yaml)",
    "    --version <semver>                  Release version to verify",
    "    --channel <name>                    Release channel to verify",
    "  verify-all [--registry <path>]        Verify every configured registry channel",
  ].join("\n");
}

function parseInputs(values: string[]): Record<string, string> {
  const inputs: Record<string, string> = {};
  for (const item of values) {
    const eq = item.indexOf("=");
    if (eq === -1) throw new OPFError(`invalid --input ${JSON.stringify(item)}, expected key=value`);
    inputs[item.slice(0, eq)] = item.slice(eq + 1);
  }
  return inputs;
}

function takeFlagValue(args: string[], index: number, flag: string): { value: string; next: number } {
  const value = args[index + 1];
  if (value === undefined || value.startsWith("-")) throw new OPFError(`missing value for ${flag}`);
  return { value, next: index + 1 };
}

export async function runCli(argv: string[]): Promise<{ stdout: string; exitCode: number }> {
  if (argv.length === 0 || argv[0] === "-h" || argv[0] === "--help") {
    return { stdout: usage(), exitCode: 0 };
  }
  const [command, ...rest] = argv;

  if (command === "validate") {
    if (rest.length !== 1) throw new OPFError("usage: opf validate <file>");
    const prompt = await load(rest[0]);
    return { stdout: `valid: ${prompt.metadata.id} (${prompt.sourceDigest})`, exitCode: 0 };
  }

  if (command === "validate-collection") {
    if (rest.length !== 1) throw new OPFError("usage: opf validate-collection <dir>");
    const prompts = await loadCollection(rest[0]);
    return { stdout: `valid: ${prompts.length} prompt(s)`, exitCode: 0 };
  }

  if (command === "compat") {
    let file: string | undefined;
    let format = "text";
    for (let i = 0; i < rest.length; i += 1) {
      const arg = rest[i];
      if (arg === "--format") {
        const parsed = takeFlagValue(rest, i, "--format");
        format = parsed.value;
        i = parsed.next;
      } else if (arg.startsWith("-")) {
        throw new OPFError(`unknown option: ${arg}`);
      } else if (!file) {
        file = arg;
      } else {
        throw new OPFError(`unexpected argument: ${arg}`);
      }
    }
    if (!file || (format !== "text" && format !== "json")) {
      throw new OPFError("usage: opf compat <file> [--format json|text]");
    }
    const prompt = await load(file);
    const report = prompt.compatibility();
    if (format === "json") return { stdout: JSON.stringify(report, null, 2), exitCode: 0 };
    const compatible = !report.findings.some((f) => f.severity === "error");
    const lines = [
      `id: ${prompt.metadata.id}`,
      `format: ${prompt.metadata.format}`,
      `source_kind: ${report.source_kind}`,
      `target_kind: ${report.target_kind}`,
      `compatible: ${compatible ? "yes" : "no"}`,
      ...report.findings.map(
        (f) => `[${f.severity}${f.category ? `/${f.category}` : ""}] ${f.code}${f.source_line ? ` (line ${f.source_line})` : ""}: ${f.message}`,
      ),
    ];
    return { stdout: lines.join("\n"), exitCode: 0 };
  }

  if (command === "render") {
    let target: string | undefined;
    let registryPath: string | undefined;
    let version: string | undefined;
    let channel: string | undefined;
    let receipt = false;
    const rawInputs: string[] = [];
    for (let i = 0; i < rest.length; i += 1) {
      const arg = rest[i];
      if (arg === "--input") {
        const parsed = takeFlagValue(rest, i, "--input");
        rawInputs.push(parsed.value);
        i = parsed.next;
      } else if (arg === "--registry") {
        const parsed = takeFlagValue(rest, i, "--registry");
        registryPath = parsed.value;
        i = parsed.next;
      } else if (arg === "--version") {
        const parsed = takeFlagValue(rest, i, "--version");
        version = parsed.value;
        i = parsed.next;
      } else if (arg === "--channel") {
        const parsed = takeFlagValue(rest, i, "--channel");
        channel = parsed.value;
        i = parsed.next;
      } else if (arg === "--receipt") {
        receipt = true;
      } else if (arg.startsWith("-")) {
        throw new OPFError(`unknown option: ${arg}`);
      } else if (!target) {
        target = arg;
      } else {
        throw new OPFError(`unexpected argument: ${arg}`);
      }
    }
    if (!target) throw new OPFError("usage: opf render <target> [--input k=v ...]");
    const inputs = parseInputs(rawInputs);
    if (registryPath || version || channel) {
      const registry = await Registry.local(registryPath ?? "opf.yaml");
      const registered = await registry.get(target, { version, channel });
      const prepared = registered.render(inputs);
      const output = receipt ? { messages: prepared.messages, receipt: prepared.receipt } : prepared.messages;
      return { stdout: JSON.stringify(output, null, 2), exitCode: 0 };
    }
    const prompt = await load(target);
    return { stdout: JSON.stringify(prompt.render(inputs), null, 2), exitCode: 0 };
  }

  if (command === "verify") {
    let id: string | undefined;
    let registryPath = "opf.yaml";
    let version: string | undefined;
    let channel: string | undefined;
    for (let i = 0; i < rest.length; i += 1) {
      const arg = rest[i];
      if (arg === "--registry") {
        const parsed = takeFlagValue(rest, i, "--registry");
        registryPath = parsed.value;
        i = parsed.next;
      } else if (arg === "--version") {
        const parsed = takeFlagValue(rest, i, "--version");
        version = parsed.value;
        i = parsed.next;
      } else if (arg === "--channel") {
        const parsed = takeFlagValue(rest, i, "--channel");
        channel = parsed.value;
        i = parsed.next;
      } else if (arg.startsWith("-")) {
        throw new OPFError(`unknown option: ${arg}`);
      } else if (!id) {
        id = arg;
      } else {
        throw new OPFError(`unexpected argument: ${arg}`);
      }
    }
    if (!id) throw new OPFError("usage: opf verify <id> [--registry opf.yaml] [--version X | --channel Y]");
    const registry = await Registry.local(registryPath);
    const registered = await registry.get(id, { version, channel });
    return {
      stdout: JSON.stringify(
        { id, version: registered.version ?? null, channel: channel ?? null, digest: registered.bundle.digest },
        null,
        2,
      ),
      exitCode: 0,
    };
  }

  if (command === "verify-all") {
    let registryPath = "opf.yaml";
    for (let i = 0; i < rest.length; i += 1) {
      const arg = rest[i];
      if (arg === "--registry") {
        const parsed = takeFlagValue(rest, i, "--registry");
        registryPath = parsed.value;
        i = parsed.next;
      } else {
        throw new OPFError(`unknown argument: ${arg}`);
      }
    }
    const registry = await Registry.local(registryPath);
    const results: Array<{ id: string; channel: string; version: string; digest: string }> = [];
    for (const [id, entry] of Object.entries(registry.config.prompts)) {
      for (const channel of Object.keys(entry.channels ?? {})) {
        const registered = await registry.get(id, { channel });
        results.push({ id, channel, version: registered.version ?? "", digest: registered.bundle.digest });
      }
    }
    return { stdout: JSON.stringify(results, null, 2), exitCode: 0 };
  }

  throw new OPFError(`unknown command: ${command}`);
}

const isMain = process.argv[1] && new URL(import.meta.url).pathname === process.argv[1];
if (isMain) {
  runCli(process.argv.slice(2))
    .then((result) => {
      if (result.stdout) console.log(result.stdout);
      process.exitCode = result.exitCode;
    })
    .catch((error: unknown) => {
      const message = error instanceof Error ? error.message : String(error);
      console.error(`error: ${message}`);
      process.exitCode = 1;
    });
}
