#!/usr/bin/env node
import {
  cpSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  renameSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { createHash } from "node:crypto";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptPath = fileURLToPath(import.meta.url);
const pluginRoot = path.resolve(path.dirname(scriptPath), "..");
const templatesRoot = path.join(pluginRoot, "templates");

const availableTemplates = new Set(["prototype", "mobile-app"]);

function parseArgs(argv) {
  const args = {};
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (!arg.startsWith("--")) continue;
    const raw = arg.slice(2);
    const [key, inlineValue] = raw.split("=", 2);
    if (inlineValue !== undefined) {
      args[key] = inlineValue;
      continue;
    }
    const next = argv[index + 1];
    if (next && !next.startsWith("--")) {
      args[key] = next;
      index += 1;
    } else {
      args[key] = true;
    }
  }
  return args;
}

function slugify(value) {
  return (
    String(value || "prototype")
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-|-$/g, "") || "prototype"
  );
}

function readText(filePath) {
  return existsSync(filePath) ? readFileSync(filePath, "utf8") : "";
}

function writeText(filePath, value) {
  mkdirSync(path.dirname(filePath), { recursive: true });
  writeFileSync(filePath, value, "utf8");
}

function copyDir(source, target) {
  cpSync(source, target, {
    recursive: true,
    force: true,
    filter(current) {
      const name = path.basename(current);
      return !["node_modules", "dist", ".vite", ".turbo", ".DS_Store"].includes(name);
    },
  });
}

function hasFiles(dir) {
  return existsSync(dir) && readdirSync(dir).length > 0;
}

function ensureEmptyDest(dest) {
  if (existsSync(dest) && statSync(dest).isFile()) {
    throw new Error(`Destination exists and is not a directory: ${dest}`);
  }
  if (hasFiles(dest)) {
    throw new Error(`Destination exists and is not empty: ${dest}`);
  }
  mkdirSync(dest, { recursive: true });
}

function nearestPrototypeRoot(args) {
  const root = args.root || args.dest || process.cwd();
  return path.resolve(root);
}

function selectedTemplate(args) {
  const template = String(args.template || "prototype").trim();
  if (!availableTemplates.has(template)) {
    throw new Error(
      `Unknown template: ${template}. Available templates: ${Array.from(availableTemplates).join(", ")}`,
    );
  }
  return template;
}

function packageJsonPath(root) {
  return path.join(root, "package.json");
}

function readPackage(root) {
  const filePath = packageJsonPath(root);
  if (!existsSync(filePath)) return null;
  return JSON.parse(readText(filePath));
}

function writePackage(root, pkg) {
  writeText(packageJsonPath(root), `${JSON.stringify(pkg, null, 2)}\n`);
}

function setPackageName(root) {
  const pkg = readPackage(root);
  if (!pkg) return;
  pkg.name = slugify(path.basename(root));
  writePackage(root, pkg);
}

function setNpmDefaults(root) {
  writeText(path.join(root, ".npmrc"), [
    "fund=false",
    "audit=false",
    "",
  ].join("\n"));
}

function createNew(root, template) {
  const templateRoot = path.join(templatesRoot, template);
  if (!existsSync(templateRoot)) {
    throw new Error(`Bundled Product Design template is missing: ${templateRoot}`);
  }
  ensureEmptyDest(root);
  copyDir(templateRoot, root);
  setPackageName(root);
  setNpmDefaults(root);
  const dependenciesReady = installBundledDependencies(root, template);
  return { status: "created", root, template, dependenciesReady };
}

function sha256(filePath) {
  return createHash("sha256").update(readFileSync(filePath)).digest("hex");
}

function resolveBundledDependencies(template) {
  const codexHome = process.env.CODEX_HOME?.trim();
  if (!codexHome) return null;

  const pluginManifest = JSON.parse(readText(path.join(pluginRoot, ".codex-plugin", "plugin.json")));
  const version = String(pluginManifest.version || "").trim();
  if (!version) throw new Error("Product Design plugin version is missing; reinstall Skill Hub.");

  const sourceTemplate = path.join(templatesRoot, template);
  const packageHash = sha256(path.join(sourceTemplate, "package.json"));
  const lockHash = sha256(path.join(sourceTemplate, "package-lock.json"));
  const runtimeBase = path.join(codexHome, "product-design-runtime");
  if (!existsSync(runtimeBase)) {
    throw new Error("Offline Product Design dependencies are missing. Reinstall Product Design from Skill Hub.");
  }

  let matchingBundle = null;
  for (const entry of readdirSync(runtimeBase, { withFileTypes: true })) {
    if (!entry.isDirectory() || !entry.name.startsWith(`${version}-`)) continue;
    const runtimeRoot = path.join(runtimeBase, entry.name);
    const dependencyManifestPath = path.join(runtimeRoot, "product-design-dependencies.json");
    if (!existsSync(dependencyManifestPath)) continue;
    let dependencyManifest;
    try {
      dependencyManifest = JSON.parse(readText(dependencyManifestPath));
    } catch {
      continue;
    }
    const expected = dependencyManifest.templates?.[template];
    const nodeModules = path.join(runtimeRoot, template, "node_modules");
    if (dependencyManifest.pluginVersion !== version || dependencyManifest.platform !== "win32-x64" ||
        dependencyManifest.nodeVersion !== "22.22.0" || !expected ||
        expected.packageJsonSha256 !== packageHash || expected.packageLockSha256 !== lockHash ||
        !existsSync(nodeModules)) continue;
    matchingBundle = { nodeModules, expectedFileCount: Number(expected.nodeModulesFiles) };
    break;
  }
  if (!matchingBundle) {
    throw new Error(`Offline dependencies do not match the ${template} template. Reinstall Product Design from Skill Hub.`);
  }

  const essentialPackage = template === "prototype" ? "vite" : "@vitejs/plugin-react";
  if (!existsSync(path.join(matchingBundle.nodeModules, essentialPackage, "package.json"))) {
    throw new Error(`Offline ${template} dependencies are incomplete. Reinstall Skill Hub.`);
  }
  return matchingBundle;
}

function copyDependencyTree(source, destination) {
  mkdirSync(destination, { recursive: true });
  let fileCount = 0;
  for (const entry of readdirSync(source, { withFileTypes: true })) {
    const sourcePath = path.join(source, entry.name);
    const destinationPath = path.join(destination, entry.name);
    if (entry.isSymbolicLink()) {
      throw new Error(`Offline dependency bundle contains an unsupported symbolic link: ${sourcePath}`);
    }
    if (entry.isDirectory()) {
      fileCount += copyDependencyTree(sourcePath, destinationPath);
    } else if (entry.isFile()) {
      copyFileSync(sourcePath, destinationPath);
      fileCount += 1;
    }
  }
  return fileCount;
}

function copyBundledDependencies(root, template) {
  const bundle = resolveBundledDependencies(template);
  if (!bundle) return false;

  const destination = path.join(root, "node_modules");
  if (existsSync(destination)) {
    throw new Error(`node_modules already exists; refusing to overwrite: ${destination}`);
  }

  const staging = path.join(root, `.product-design-node-modules-${process.pid}`);
  if (existsSync(staging)) {
    throw new Error(`Dependency staging directory already exists: ${staging}`);
  }
  const copiedFiles = copyDependencyTree(bundle.nodeModules, staging);
  if (copiedFiles !== bundle.expectedFileCount) {
    throw new Error(`Offline dependency copy was incomplete: expected ${bundle.expectedFileCount} files, copied ${copiedFiles}.`);
  }
  renameSync(staging, destination);
  return true;
}

function installBundledDependencies(root, template) {
  return copyBundledDependencies(root, template);
}

function restoreDependencies(root, template) {
  if (!existsSync(root) || !statSync(root).isDirectory()) {
    throw new Error(`Prototype project directory does not exist: ${root}`);
  }
  const templateRoot = path.join(templatesRoot, template);
  const lockPath = path.join(root, "package-lock.json");
  const templateLockPath = path.join(templateRoot, "package-lock.json");
  if (!existsSync(lockPath) || sha256(lockPath) !== sha256(templateLockPath)) {
    throw new Error(`Project lockfile does not match the bundled ${template} template; refusing to replace dependencies.`);
  }

  if (!process.env.CODEX_HOME?.trim()) {
    throw new Error("Offline Product Design dependencies are missing. Reinstall Product Design from Skill Hub.");
  }
  copyBundledDependencies(root, template);
  return { status: "dependencies-restored", root, template };
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const root = nearestPrototypeRoot(args);

  const mode = String(args.mode || "new");
  const template = selectedTemplate(args);
  const result = mode === "new"
    ? createNew(root, template)
    : mode === "restore-dependencies"
      ? restoreDependencies(root, template)
      : (() => { throw new Error(`Unknown mode: ${mode}`); })();
  console.log(JSON.stringify(result, null, 2));
}

try {
  main();
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
}
