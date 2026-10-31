// Builds customwin-helper in release mode and copies it where Tauri expects a sidecar:
// src-tauri/binaries/customwin-helper-<target-triple>.exe
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const triple =
  process.env.TAURI_ENV_TARGET_TRIPLE ??
  execFileSync("rustc", ["-vV"], { encoding: "utf8" }).match(/^host: (\S+)$/m)[1];
const ext = triple.includes("windows") ? ".exe" : "";

execFileSync("cargo", ["build", "--release", "-p", "customwin-helper", "--target", triple], { cwd: root, stdio: "inherit" });

const outDir = join(root, "app", "src-tauri", "binaries");
mkdirSync(outDir, { recursive: true });
copyFileSync(join(root, "target", triple, "release", `customwin-helper${ext}`), join(outDir, `customwin-helper-${triple}${ext}`));
console.log(`helper ready for ${triple}`);
