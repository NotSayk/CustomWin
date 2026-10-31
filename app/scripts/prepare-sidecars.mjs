// Builds the native executables shipped next to the app (Tauri "sidecars") in release
// mode and copies them where Tauri expects them:
// src-tauri/binaries/<name>-<target-triple>.exe
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const SIDECARS = ["customwin-helper", "customwin-desktop"];

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const triple =
  process.env.TAURI_ENV_TARGET_TRIPLE ??
  execFileSync("rustc", ["-vV"], { encoding: "utf8" }).match(/^host: (\S+)$/m)[1];
const ext = triple.includes("windows") ? ".exe" : "";

const packages = SIDECARS.flatMap((name) => ["-p", name]);
execFileSync("cargo", ["build", "--release", ...packages, "--target", triple], { cwd: root, stdio: "inherit" });

const outDir = join(root, "app", "src-tauri", "binaries");
mkdirSync(outDir, { recursive: true });
for (const name of SIDECARS) {
  copyFileSync(join(root, "target", triple, "release", `${name}${ext}`), join(outDir, `${name}-${triple}${ext}`));
}
console.log(`sidecars ready for ${triple}: ${SIDECARS.join(", ")}`);
