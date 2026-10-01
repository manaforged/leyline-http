#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import path from "node:path";

const args = process.argv.slice(2);
let base = null;
let head = "HEAD";
let explain = false;
let gates = false;
let json = false;
let filesOverride = null;
for (let i = 0; i < args.length; i += 1) {
  if (args[i] === "--base") base = args[++i];
  else if (args[i] === "--head") head = args[++i];
  else if (args[i] === "--files") filesOverride = args[++i].split(",").filter(Boolean);
  else if (args[i] === "--explain") explain = true;
  else if (args[i] === "--json") json = true;
  else if (args[i] === "--gates") gates = true;
  else {
    process.stderr.write(`affected-packages: unknown argument ${args[i]}\n`);
    process.exit(2);
  }
}

const root = execFileSync("git", ["rev-parse", "--show-toplevel"], { encoding: "utf8" }).trim();
const metadata = JSON.parse(
  execFileSync("cargo", ["metadata", "--format-version", "1", "--locked"], {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  }),
);
const workspaceIds = new Set(metadata.workspace_members);
const packages = metadata.packages.filter((pkg) => workspaceIds.has(pkg.id));
const byId = new Map(packages.map((pkg) => [pkg.id, pkg]));
const roots = packages
  .map((pkg) => ({
    id: pkg.id,
    name: pkg.name,
    dir: path.relative(root, path.dirname(pkg.manifest_path)).replaceAll(path.sep, "/"),
  }))
  .sort((a, b) => b.dir.length - a.dir.length || a.name.localeCompare(b.name));

let full = filesOverride === null && (!base || /^0+$/.test(base));
let changed = filesOverride ?? [];
if (!full && filesOverride === null) {
  try {
    changed = execFileSync("git", ["diff", "--name-only", "--diff-filter=ACMRD", base, head, "--"], {
      cwd: root,
      encoding: "utf8",
    })
      .trim()
      .split("\n")
      .filter(Boolean);
  } catch {
    process.stderr.write(`affected-packages: cannot diff ${base}..${head}; selecting full workspace\n`);
    full = true;
  }
}
changed = [...new Set(changed.map((file) => file.replaceAll("\\", "/")))].sort();

const GLOBAL_RUST_INPUTS = new Set([
  "Cargo.toml",
  "Cargo.lock",
  "rust-toolchain.toml",
  "rust-toolchain",
  "clippy.toml",
  "rustfmt.toml",
  ".rustfmt.toml",
  "deny.toml",
  ".config/nextest.toml",
]);

const OWNED_PATHS = [
  { path: "crates/leyline/README.md", package: "leyline-http" },
  { path: "docs/", package: "leyline-http" },
];

const PR_GATES = [
  { gate: "deny", pattern: /(^|\/)Cargo\.(toml|lock)$|^deny\.toml$/ },
  { gate: "msrv", pattern: /(^|\/)Cargo\.(toml|lock)$|^rust-toolchain(\.toml)?$/, anyPackage: true },
  { gate: "package", pattern: /(^|\/)Cargo\.(toml|lock)$|^scripts\/stage-package-workspace\.mjs$/, anyPackage: true },
  { gate: "subcrates", pattern: /^crates\/leyline-bssl(-sys|-tokio)?\// },
  { gate: "semver", packages: ["leyline-http"] },
];

const CI_LOGIC_FILES = new Set([
  "scripts/affected-packages.mjs",
  "scripts/verify.sh",
  "scripts/feature-matrix.sh",
]);

const ownedBy = (file) =>
  OWNED_PATHS.find((entry) =>
    entry.path.endsWith("/") ? file.startsWith(entry.path) : file === entry.path,
  );

const isDoc = (file) =>
  file.endsWith(".md") || file.startsWith("docs/") || file.startsWith("book/");

const IGNORED_PREFIXES = [".github/", "scripts/", "docs/", "book/"];
const IGNORED_FILES = new Set([
  ".gitignore",
  ".gitattributes",
  "CHANGELOG.md",
  "LICENSE",
  "NOTICE",
  "SECURITY.md",
]);

const docsOnly =
  !full && changed.length > 0 && changed.every(isDoc) && !changed.some(ownedBy);
const workflows = changed.some(
  (file) => file.startsWith(".github/workflows/") || file === ".github/actionlint.yaml",
);
const dependencyAudit = changed.some(
  (file) => /(^|\/)Cargo\.(toml|lock)$/.test(file) || file === "deny.toml",
);

if (
  changed.some(
    (file) =>
      GLOBAL_RUST_INPUTS.has(file) || CI_LOGIC_FILES.has(file) || file.startsWith(".cargo/"),
  )
)
  full = true;

const direct = new Set();
if (!full) {
  for (const file of changed) {
    const owned = ownedBy(file);
    if (owned) {
      const pkg = roots.find((entry) => entry.name === owned.package);
      if (pkg) {
        direct.add(pkg.id);
        continue;
      }
      full = true;
      break;
    }
    if (isDoc(file)) continue;
    if (IGNORED_PREFIXES.some((prefix) => file.startsWith(prefix)) || IGNORED_FILES.has(file)) continue;
    const owner = roots.find((pkg) =>
      pkg.dir === "" ? true : file === pkg.dir || file.startsWith(`${pkg.dir}/`),
    );
    if (owner) {
      direct.add(owner.id);
      continue;
    }
    full = true;
    break;
  }
}

function close(seed, reverse) {
  const selected = new Set(seed);
  const queue = [...selected];
  while (queue.length) {
    const id = queue.shift();
    for (const dependent of reverse.get(id) ?? []) {
      if (!selected.has(dependent)) {
        selected.add(dependent);
        queue.push(dependent);
      }
    }
  }
  return selected;
}

let selected = full ? new Set(workspaceIds) : new Set(direct);
if (!full && metadata.resolve?.nodes) {
  const reverse = new Map();
  for (const node of metadata.resolve.nodes) {
    if (!workspaceIds.has(node.id)) continue;
    for (const dep of node.deps ?? []) {
      if (!workspaceIds.has(dep.pkg)) continue;
      if (!reverse.has(dep.pkg)) reverse.set(dep.pkg, new Set());
      reverse.get(dep.pkg).add(node.id);
    }
  }
  selected = close(direct, reverse);
}

const names = [...selected].map((id) => byId.get(id)?.name).filter(Boolean).sort();
const plan = {
  mode: full ? "full" : docsOnly ? "docs" : "affected",
  files: changed,
  full,
  docsOnly,
  packages: names,
  workflows,
  dependencyAudit,
  gates: PR_GATES.filter(
    ({ pattern, anyPackage, packages }) =>
      full ||
      (pattern && changed.some((file) => pattern.test(file))) ||
      (anyPackage && names.length > 0) ||
      (packages && packages.some((name) => names.includes(name))),
  ).map(({ gate }) => gate),
};

if (explain) {
  process.stderr.write(
    `affected-packages: ${full ? "full workspace" : `${direct.size} direct -> ${names.length} with reverse dependencies`}\n`,
  );
  if (changed.length) process.stderr.write(`changed files: ${changed.length}\n`);
  process.stderr.write(`packages: ${names.length ? names.join(", ") : "none (non-Rust change)"}\n`);
}
if (gates) process.stdout.write(plan.gates.length ? `${plan.gates.join(",")}\n` : "");
else process.stdout.write(json ? `${JSON.stringify(plan)}\n` : names.length ? `${names.join("\n")}\n` : "");
