import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, it } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));
const script = resolve(here, "skill-hub-relay-sync.py");
const config = JSON.parse(readFileSync(resolve(here, "skill-hub-relay-config.json"), "utf8"));
const edge = readFileSync(resolve(here, "codex-update-relay-edge.conf"), "utf8");
const service = readFileSync(resolve(here, "skill-hub-relay-sync.service"), "utf8");
const timer = readFileSync(resolve(here, "skill-hub-relay-sync.timer"), "utf8");
const source = readFileSync(script, "utf8");
const pythonCommand = process.env.PYTHON?.trim() || (process.platform === "win32" ? "python" : "python3");

let pythonAvailable = true;
try {
  execFileSync(pythonCommand, ["--version"], { stdio: "ignore" });
} catch (error) {
  if (error?.code === "ENOENT") {
    pythonAvailable = false;
    console.warn(`SKIP Python syntax execution: ${pythonCommand} is not available; static relay assertions still run.`);
  } else {
    throw error;
  }
}

describe("skill-hub relay contract", () => {
  it("keeps the fixed source, namespace, security boundaries, and atomic publication", () => {
    assert.deepEqual(Object.keys(config).sort(), [
      "branch",
      "exportRoot",
      "githubArchiveTemplate",
      "githubCommitUrl",
      "lockFile",
      "maxPublishedReleases",
      "repository",
      "stateRoot",
      "workRoot",
    ].sort());
    assert.equal(config.repository, "meta-xucong/skill_hub");
    assert.equal(config.branch, "master");
    assert.equal(config.githubCommitUrl, "https://api.github.com/repos/meta-xucong/skill_hub/commits/master");
    assert.equal(config.githubArchiveTemplate, "https://github.com/meta-xucong/skill_hub/archive/{commit}.zip");
    assert.equal(config.exportRoot, "/srv/codex-update-relay/export");
    assert.equal(config.workRoot, "/srv/codex-update-relay/work/skill-hub");
    assert.equal(config.stateRoot, "/srv/codex-update-relay/state");
    assert.equal(config.lockFile, "/srv/codex-update-relay/state/skill-hub-sync.lock");
    assert.equal(config.maxPublishedReleases, 5);

    assert.match(source, /EXPECTED_REPOSITORY = "meta-xucong\/skill_hub"/);
    assert.match(source, /EXPECTED_GITHUB_COMMIT_URL = "https:\/\/api\.github\.com\/repos\/meta-xucong\/skill_hub\/commits\/master"/);
    assert.match(source, /EXPECTED_GITHUB_ARCHIVE_TEMPLATE = "https:\/\/github\.com\/meta-xucong\/skill_hub\/archive\/\{commit\}\.zip"/);
    assert.match(source, /EXPECTED_EXPORT_ROOT = "\/srv\/codex-update-relay\/export"/);
    assert.match(source, /METADATA_KEYS = frozenset\(\("schemaVersion", "repo", "branch", "commit", "archive", "size", "sha256"\)\)/);
    assert.match(source, /ALLOWED_GITHUB_HOSTS = frozenset\(\{"api\.github\.com", "github\.com", "codeload\.github\.com"\}\)/);
    assert.match(source, /self\.count > self\.limit/);
    assert.match(source, /validate_github_url\(newurl\)/);
    assert.match(source, /response\.read\(max_bytes \+ 1\)/);
    assert.match(source, /MAX_METADATA = 64 \* 1024/);
    assert.match(source, /lock\.relative_to\(roots\["stateRoot"\]\)/);
    assert.match(source, /paths_overlap\(left: Path, right: Path\)/);
    assert.match(source, /PAYLOAD_CURRENT = Path\("\/srv\/codex-update-relay\/export\/current"\)/);
    assert.match(source, /export_root = config\["exportRoot"\] \/ "skill-hub"/);
    assert.match(source, /link_target = Path\(os\.readlink\(current\)\)/);
    assert.match(source, /link_target\.is_absolute\(\) or link_target\.parent != Path\("releases"\)/);
    assert.match(source, /set\(value\) != METADATA_KEYS/);
    assert.match(source, /archive_path = current \/ "archive\.zip"/);
    assert.match(source, /len\(archive\) != size or sha256\(archive\) != digest/);
    assert.match(source, /current_tmp\.symlink_to\(Path\("releases"\) \/ commit, target_is_directory=True\)/);
    assert.match(source, /os\.replace\(current_tmp, export_root \/ "current"\)/);
    assert.match(source, /validate_metadata\(read_json\(temporary \/ "latest\.json"\), expected\)/);
    assert.match(source, /temporary\.chmod\(0o755\)/);
    assert.match(source, /archive_path\.chmod\(0o644\)/);
    assert.match(source, /if temporary\.exists\(\):\s+shutil\.rmtree\(temporary\)/s);
    assert.match(source, /current_commit\(export_root, config\) == commit/);
  });

  it("keeps the existing Codex payload route and exposes only the two Skill Hub files", () => {
    assert.match(edge, /listen 443 ssl;/);
    assert.match(edge, /alias \/srv\/codex-update-relay\/export\/current\//);
    assert.match(edge, /location \^~ \/__codex_update__\/releases\/\s*\{\s*return 404;/s);
    assert.equal((edge.match(/location = \/__codex_update__\/skill-hub\/latest\.json \{/g) ?? []).length, 1);
    assert.equal((edge.match(/location = \/__codex_update__\/skill-hub\/archive\.zip \{/g) ?? []).length, 1);
    assert.match(edge, /location = \/__codex_update__\/skill-hub\/latest\.json \{[\s\S]*?alias \/srv\/codex-update-relay\/export\/skill-hub\/current\/latest\.json;[\s\S]*?default_type application\/json;/);
    assert.match(edge, /location = \/__codex_update__\/skill-hub\/archive\.zip \{[\s\S]*?alias \/srv\/codex-update-relay\/export\/skill-hub\/current\/archive\.zip;/);
    assert.match(edge, /location = \/__codex_update__\/skill-hub\/latest\.json \{[\s\S]*?limit_except GET HEAD \{ deny all; \}/);
    assert.match(edge, /location = \/__codex_update__\/skill-hub\/archive\.zip \{[\s\S]*?limit_except GET HEAD \{ deny all; \}/);
    assert.match(edge, /location \^~ \/__codex_update__\/skill-hub\/ \{\s*return 404;\s*\}/s);
    assert.doesNotMatch(edge, /location \^~ \/__codex_update__\/skill-hub\/ \{[\s\S]*?alias/);
  });

  it("keeps the isolated service and hourly persistent timer contract", () => {
    assert.match(service, /^User=codexskillrelay$/m);
    assert.match(service, /^Group=codexskillrelay$/m);
    assert.match(service, /^WorkingDirectory=\/opt\/codex-update-relay$/m);
    assert.match(service, /^ExecStart=\/usr\/bin\/python3 \/opt\/codex-update-relay\/skill-hub-relay-sync\.py --config \/etc\/codex-update-relay\/skill-hub\.json$/m);
    assert.match(service, /^ReadWritePaths=\/srv\/codex-update-relay\/export\/skill-hub \/srv\/codex-update-relay\/work\/skill-hub \/srv\/codex-update-relay\/state$/m);
    assert.match(service, /^StandardOutput=journal$/m);
    assert.match(service, /^StandardError=journal$/m);
    assert.doesNotMatch(service, /ReadWritePaths=.*\/srv\/codex-update-relay\/export\/current/);
    assert.match(timer, /^OnCalendar=hourly$/m);
    assert.match(timer, /^Persistent=true$/m);
    assert.match(timer, /^Unit=skill-hub-relay-sync\.service$/m);
  });

  it.skipIf(!pythonAvailable)("passes Python syntax validation without creating bytecode", () => {
    execFileSync(pythonCommand, [
      "-B",
      "-c",
      "import sys; compile(open(sys.argv[1], 'rb').read(), sys.argv[1], 'exec')",
      script,
    ], { stdio: "inherit" });
  });
});
