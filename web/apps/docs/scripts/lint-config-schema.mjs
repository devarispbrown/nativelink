#!/usr/bin/env node
// Every NativeLink config shipped in the repository must validate against
// the JSON Schema generated from the Rust config types.
//
// The schema is what editors use to autocomplete and check a config. If it
// rejects a config the server accepts, editors flag valid files and people
// learn to ignore the warnings. The shipped examples are the best sample of
// real configs we have, so they are the test.
//
// Usage, from web/:
//   pnpm run --filter @nativelink/docs lint:config-schema
// or directly, with a schema that's already built:
//   node scripts/lint-config-schema.mjs --schema path/to/nativelink_config.schema.json
//
// Without --schema this runs the `build-schema` binary, the same way
// gen-config-reference.mjs does, so it needs a Rust toolchain.
//
// A file counts as a NativeLink config if it parses as JSON5 and has a
// top-level `stores` or `servers` key; other JSON files in those
// directories (worker property files, for example) are skipped.
//
// Exit code is 1 if any config fails to validate.

import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import Ajv2020 from "ajv/dist/2020.js";
import JSON5 from "json5";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = execFileSync("git", ["rev-parse", "--show-toplevel"], {
  encoding: "utf8",
  cwd: here,
}).trim();

const CONFIG_DIRS = ["nativelink-config/examples", "deployment-examples", "integration_tests"];
const SCHEMA_FILE = "nativelink_config.schema.json";

function buildSchema() {
  // build-schema writes into its working directory, so give it a scratch
  // one instead of the repository root.
  const outDir = mkdtempSync(join(tmpdir(), "nl-schema-"));
  try {
    execFileSync(
      "cargo",
      [
        "run",
        "--quiet",
        "--manifest-path",
        join(repoRoot, "Cargo.toml"),
        "--bin",
        "build-schema",
        "--features",
        "dev-schema",
        "--package",
        "nativelink-config",
      ],
      { cwd: outDir, stdio: ["ignore", "inherit", "inherit"] },
    );
    return JSON.parse(readFileSync(join(outDir, SCHEMA_FILE), "utf8"));
  } finally {
    rmSync(outDir, { recursive: true, force: true });
  }
}

function loadSchema() {
  const flag = process.argv.indexOf("--schema");
  if (flag !== -1) {
    const path = process.argv[flag + 1];
    if (!path) throw new Error("--schema needs a path");
    return JSON.parse(readFileSync(path, "utf8"));
  }
  return buildSchema();
}

function configFiles() {
  const files = [];
  const walk = (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) walk(path);
      else if (/\.json5?$/.test(entry.name)) files.push(path);
    }
  };
  for (const dir of CONFIG_DIRS) walk(join(repoRoot, dir));
  return files.sort();
}

/** The most specific errors, one line each. anyOf and oneOf summaries only
 *  repeat what their branches already said. */
function describe(errors) {
  const seen = new Set();
  return errors
    .filter((e) => e.keyword !== "anyOf" && e.keyword !== "oneOf")
    .map((e) => {
      const extra = e.params?.additionalProperty ? ` (\`${e.params.additionalProperty}\`)` : "";
      return `${e.instancePath || "/"} ${e.message}${extra}`;
    })
    .filter((line) => !seen.has(line) && seen.add(line));
}

function main() {
  // schemars emits integer formats such as "uint32" that ajv does not know;
  // the type and minimum already carry the meaning.
  const ajv = new Ajv2020({
    allErrors: true,
    strict: false,
    validateFormats: false,
  });
  const validate = ajv.compile(loadSchema());

  let checked = 0;
  let skipped = 0;
  const failures = [];
  for (const file of configFiles()) {
    const name = relative(repoRoot, file);
    let config;
    try {
      config = JSON5.parse(readFileSync(file, "utf8"));
    } catch (err) {
      failures.push(`${name}: not valid JSON5: ${err.message}`);
      continue;
    }
    if (
      typeof config !== "object" ||
      config === null ||
      !("stores" in config || "servers" in config)
    ) {
      skipped += 1;
      continue;
    }
    checked += 1;
    if (!validate(config)) {
      failures.push(
        `${name}:\n${describe(validate.errors)
          .slice(0, 10)
          .map((line) => `    ${line}`)
          .join("\n")}`,
      );
    }
  }

  if (checked === 0) {
    console.error("Found no NativeLink configs to check; is the walk broken?");
    process.exit(1);
  }
  if (failures.length > 0) {
    console.error(`${failures.length} config(s) don't match the generated schema.`);
    console.error(
      "Either the config is wrong, or the schema doesn't describe something the server accepts (see nativelink-config/src/schema.rs).\n",
    );
    for (const failure of failures) console.error(failure);
    process.exit(1);
  }
  console.log(
    `Checked ${checked} NativeLink config(s) against the generated schema; ` +
      `skipped ${skipped} other JSON file(s).`,
  );
}

main();
