import test from "node:test";
import assert from "node:assert/strict";
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
  rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { parse, stringify } from "smol-toml";

test("release strips the app but not host proc-macros or build scripts", () => {
  const manifest = parse(
    readFileSync(new URL("../src-tauri/Cargo.toml", import.meta.url), "utf8"),
  );
  const root = mkdtempSync(path.join(tmpdir(), "llm-release-profile-"));
  try {
    mkdirSync(path.join(root, "src"));
    mkdirSync(path.join(root, "macro", "src"), { recursive: true });
    writeFileSync(
      path.join(root, "Cargo.toml"),
      stringify({
        package: { name: "profile-app", version: "0.0.0", edition: "2021" },
        dependencies: { "profile-macro": { path: "macro" } },
        profile: { release: manifest.profile.release },
      }),
    );
    writeFileSync(path.join(root, "build.rs"), "fn main() {}\n");
    writeFileSync(
      path.join(root, "src", "main.rs"),
      '#[profile_macro::passthrough]\nfn main() { println!("profile-ok"); }\n',
    );
    writeFileSync(
      path.join(root, "macro", "Cargo.toml"),
      '[package]\nname="profile-macro"\nversion="0.0.0"\nedition="2021"\n[lib]\nproc-macro=true\n',
    );
    writeFileSync(
      path.join(root, "macro", "src", "lib.rs"),
      "extern crate proc_macro;\n#[proc_macro_attribute]\npub fn passthrough(_: proc_macro::TokenStream, item: proc_macro::TokenStream) -> proc_macro::TokenStream { item }\n",
    );
    const env = { ...process.env, CARGO_TARGET_DIR: path.join(root, "target") };
    // Test the checked-in defaults, not a developer's profile/rustflags workaround.
    for (const key of Object.keys(env)) {
      if (key.startsWith("CARGO_PROFILE_") || key.endsWith("RUSTFLAGS"))
        delete env[key];
    }
    const build = spawnSync(
      "pnpm",
      [
        "rust",
        "--",
        "build",
        "--offline",
        "--release",
        "--verbose",
        "--target-dir",
        path.join(root, "target"),
        "--manifest-path",
        path.join(root, "Cargo.toml"),
      ],
      {
        cwd: new URL("..", import.meta.url),
        env,
        encoding: "utf8",
        timeout: 120_000,
      },
    );
    assert.equal(build.status, 0, build.error?.message ?? build.stderr);
    const commands = build.stderr.split(/\r?\n/);
    for (const crate of [
      "profile_macro",
      "build_script_build",
      "profile_app",
    ]) {
      const command = commands.find((line) =>
        line.includes(`--crate-name ${crate} `),
      );
      assert.ok(command, `missing rustc command for ${crate}: ${build.stderr}`);
      if (crate === "profile_app") assert.match(command, /strip=symbols/);
      else assert.doesNotMatch(command, /strip=(symbols|debuginfo)/);
    }
    const app = spawnSync(
      path.join(
        root,
        "target",
        "release",
        `profile-app${process.platform === "win32" ? ".exe" : ""}`,
      ),
      { encoding: "utf8" },
    );
    assert.equal(app.status, 0, app.stderr);
    assert.equal(app.stdout.trim(), "profile-ok");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
