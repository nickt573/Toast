import { execFileSync, execSync } from "node:child_process";
import { existsSync, readdirSync, rmSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const isWindows = process.platform === "win32";
const sdk = process.env.ANDROID_HOME || process.env.ANDROID_SDK_ROOT || "";

function step(label, fn) {
  try {
    const detail = fn();
    console.log(`  ok   ${label}${detail ? ` (${detail})` : ""}`);
  } catch (err) {
    console.log(`  skip ${label} (${err.message.split("\n")[0]})`);
  }
}

function killByName(names) {
  let hit = 0;
  for (const name of names) {
    try {
      if (isWindows) execSync(`taskkill /F /IM "${name}" /T`, { stdio: "ignore" });
      else execSync(`pkill -f "${name}"`, { stdio: "ignore" });
      hit++;
    } catch {}
  }
  return hit ? `${hit} matched` : "none running";
}

function killEmulators() {
  if (isWindows) {
    return killByName([
      "qemu-system-x86_64.exe",
      "qemu-system-aarch64.exe",
      "emulator.exe",
      "emulator64-crash-service.exe",
      "crashpad_handler.exe",
    ]);
  }
  return killByName(["qemu-system", "emulator/emulator", "emulator64-", "crashpad_handler"]);
}

function killAdbServer() {
  const adb = sdk ? join(sdk, "platform-tools", isWindows ? "adb.exe" : "adb") : "adb";
  execFileSync(existsSync(adb) ? adb : "adb", ["kill-server"], { stdio: "ignore" });
  return "adb kill-server";
}

function stopGradle() {
  const wrapper = join(
    process.cwd(),
    "src-tauri",
    "gen",
    "android",
    isWindows ? "gradlew.bat" : "gradlew",
  );
  if (!existsSync(wrapper)) return "no android project yet";
  execSync(`"${wrapper}" --stop`, {
    stdio: "ignore",
    cwd: join(process.cwd(), "src-tauri", "gen", "android"),
  });
  return "daemon stopped";
}

function avdRoot() {
  if (process.env.ANDROID_AVD_HOME) return process.env.ANDROID_AVD_HOME;
  if (process.env.ANDROID_SDK_HOME) return join(process.env.ANDROID_SDK_HOME, ".android", "avd");
  return join(homedir(), ".android", "avd");
}

function clearLocks() {
  const root = avdRoot();
  if (!existsSync(root)) return "no avd dir";
  let removed = 0;
  for (const entry of readdirSync(root)) {
    if (!entry.endsWith(".avd")) continue;
    const dir = join(root, entry);
    for (const f of readdirSync(dir)) {
      if (!f.endsWith(".lock")) continue;
      const path = join(dir, f);
      rmSync(path, { recursive: statSync(path).isDirectory(), force: true });
      removed++;
    }
  }
  return removed ? `${removed} lock(s) cleared` : "no stale locks";
}

console.log(`Ending Android emulator on ${process.platform}...`);
step("stop emulator / qemu processes", killEmulators);
step("stop adb server", killAdbServer);
step("stop gradle daemon", stopGradle);
step("clear stale AVD locks", clearLocks);
console.log("Done. The emulator and its leftovers are cleared.");
