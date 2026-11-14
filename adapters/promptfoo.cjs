const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const PROMPTFOO_INTERNAL_VARS = new Set(["__evalId", "__evalStepId", "__repeatIndex"]);

module.exports = async function renderOpfPrompt({ vars }) {
  const promptFile = process.env.OPF_PROMPT_FILE;
  if (!promptFile) {
    throw new Error("Set OPF_PROMPT_FILE to the repo-relative path of an OPF prompt file.");
  }
  const resolvedPrompt = path.resolve(process.cwd(), promptFile);
  const args = ["-m", "opf.cli", "render", resolvedPrompt];
  for (const [name, value] of Object.entries(vars ?? {})) {
    if (!PROMPTFOO_INTERNAL_VARS.has(name)) {
      args.push("--input", `${name}=${String(value)}`);
    }
  }
  const pythonSrc = path.resolve(__dirname, "../packages/python/src");
  const env = { ...process.env };
  if (fs.existsSync(pythonSrc)) {
    env.PYTHONPATH = env.PYTHONPATH ? `${pythonSrc}${path.delimiter}${env.PYTHONPATH}` : pythonSrc;
  }
  const pythonBin = process.env.OPF_PYTHON || "python3";
  const stdout = execFileSync(pythonBin, args, { encoding: "utf8", env });
  return JSON.parse(stdout);
};
