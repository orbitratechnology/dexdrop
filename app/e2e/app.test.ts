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

// Pictures of the real platform, Home in light and in dark, for the Design
// Guides' finishing checklist: saved under app/e2e-shots/ (CI moves them
// into each target's artifact). Headless runs draw nothing, so there the
// picture is skipped.
const shots = "e2e-shots";

async function picture(app: App, name: string) {
  try {
    await app.screenshot(`${shots}/${name}.png`);
  } catch {
    // Headless: only a real platform draws the app.
  }
}

type App = Awaited<ReturnType<typeof open>>;

// The app on a fresh engine and storage, at Home.
async function home(options: { colorScheme?: "light" | "dark" } = {}): Promise<App> {
  const app = await open("../nsh.json", options);
  // The engine has started once Home says how this device shows; on
  // Apple platforms the first launch then explains the local network
  // (in the same update), as it does for a person: go through it.
  await app.waitFor("text:/Visible as |· hidden$/", { timeout: 10000 });
  if (app.count("id:local-network-continue") > 0) {
    await app.press("id:local-network-continue");
    await app.waitFor("id:local-network-continue", { state: "gone" });
  }
  await app.waitFor("text:Nearby");
  return app;
}

describe("DexDrop", () => {
  let app: App;

  beforeEach(async () => {
    app = await home();
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

describe("DexDrop's look on this platform", () => {
  test("a picture of Home in light", async () => {
    const app = await home({ colorScheme: "light" });
    await app.waitFor("id:send");
    await picture(app, "home-light");
  });

  test("a picture of Home in dark", async () => {
    const app = await home({ colorScheme: "dark" });
    await app.waitFor("id:send");
    await picture(app, "home-dark");
  });
});
