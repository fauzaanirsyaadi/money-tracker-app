// Node side of tools/uia-driver: sets a field's text (or clicks, waits) through the
// accessibility API instead of `adb input text`, which drops key events when the
// emulator is busy. The driver is a separate, self-instrumenting APK
// (io.github.fauzaanirsyaadi.moneytracker.uia); it is built and installed on first use and never touches
// the app under test.
//
//   import { createUia } from "./uia.mjs";
//   const uia = createUia(adb, { root });
//   await uia.ensure();
//   uia.run("settext", { value: "493017", focused: "true" });

import { execFileSync } from "node:child_process";
import { existsSync, statSync } from "node:fs";
import path from "node:path";

const RUNNER = "io.github.fauzaanirsyaadi.moneytracker.uia/androidx.test.runner.AndroidJUnitRunner";

export function createUia(adb, { root }) {
  const apk = path.join(root, "tools/uia-driver/build/outputs/apk/debug/uia-driver-debug.apk");
  const gradlew = path.join(root, "src-tauri/gen/android/gradlew");

  const installed = () => adb("shell", "pm", "list", "instrumentation").includes("io.github.fauzaanirsyaadi.moneytracker.uia");

  return {
    /** Builds the driver when missing or older than its source, and (re)installs it. */
    ensure() {
      const source = path.join(root, "tools/uia-driver/src/main/java/io/github/fauzaanirsyaadi/moneytracker/uia/UiaDriver.kt");
      const stale = existsSync(apk) && statSync(source).mtimeMs > statSync(apk).mtimeMs;
      if (!existsSync(apk) || stale) {
        execFileSync(gradlew, ["-p", path.join(root, "tools/uia-driver"), "assembleDebug", "--console=plain", "-q"], {
          stdio: "inherit",
        });
      }
      adb("install", "-r", apk);
      if (!installed()) throw new Error("the UI driver did not install");
    },

    /** Runs one action; returns the driver's `result=` text, or throws with its failure output. */
    run(action, args = {}) {
      // Text goes as base64 so that spaces, quotes and the like survive the shell.
      if (args.value !== undefined) {
        args = { ...args, value64: Buffer.from(String(args.value), "utf8").toString("base64") };
        delete args.value;
      }
      const extra = Object.entries(args).flatMap(([key, value]) => ["-e", key, String(value)]);
      const output = adb(
        "shell", "am", "instrument", "-w", "-e", "class", "io.github.fauzaanirsyaadi.moneytracker.uia.UiaDriver",
        "-e", "action", action, ...extra, RUNNER,
      );
      const result = /INSTRUMENTATION_STATUS: result=(.*)/.exec(output);
      if (!/^OK \(1 test\)/m.test(output) || !result) {
        throw new Error(`UI driver ${action} failed: ${output.split("\n").filter((l) => /Error|Exception|failure|at com\.moneytracker/i.test(l)).slice(0, 3).join(" | ") || output.slice(-300)}`);
      }
      return result[1].trim();
    },
  };
}
