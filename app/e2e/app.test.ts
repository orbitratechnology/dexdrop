// DexDrop on a real platform, through NSH's test runner and the platform's
// own input: the same file on every target.
//
//   cd app
//   nsh test e2e/app.test.ts --target linux     # xvfb-run -a … without a display
//   nsh test e2e/app.test.ts --target macos
//   nsh test e2e/app.test.ts --target ios       # boots the newest iPhone simulator
//   nsh test e2e/app.test.ts --target android   # the first `adb devices` entry
//
// Selectors are test ids (`id:send`), set with `.test_id()` in the app, so
// they hold across platforms and wording; see docs/TESTING.md.
import { beforeEach, describe, expect, open, test } from "nsh/test";

describe("DexDrop", () => {
  let app: Awaited<ReturnType<typeof open>>;

  beforeEach(async () => {
    // A fresh engine and storage for each test.
    app = await open("../nsh.json");
    // The engine has started once Home says how this device shows; on
    // Apple platforms the first launch then explains the local network
    // (in the same update), as it does for a person: go through it.
    await app.waitFor("text:/^Visible as |· hidden$/", { timeout: 10000 });
    if (app.count("id:local-network-continue") > 0) {
      await app.press("id:local-network-continue");
      await app.waitFor("id:local-network-continue", { state: "gone" });
    }
    await app.waitFor("text:Nearby");
  });

  test("home offers Send and Receive", async () => {
    await app.waitFor("id:send");
    await app.waitFor("id:receive");
    expect(app.count("id:send")).toBe(1);
  });

  test("Send offers files, and nothing to send yet", async () => {
    await app.press("id:send");
    await app.waitFor("id:pick-files");
    // Nothing picked: no way on to a device.
    expect(app.count("id:next")).toBe(0);
    await app.back();
    await app.waitFor("id:receive");
  });

  test("Receive shows a code and an invitation to copy", async () => {
    await app.press("id:receive");
    await app.waitFor("text:Ready to receive");
    await app.waitFor("id:copy-invitation", { timeout: 10000 });
    await app.back();
    await app.waitFor("id:send");
  });
});
