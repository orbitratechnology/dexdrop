// The real app on a platform (`nsh test e2e --target linux|macos|ios|android`):
// the same selectors as the headless Rust tests, through the platform's
// own input. Headless Rust tests (app/tests) cover the rest in seconds.
import { expect, open, test } from "nsh/test";

test("switches tabs and opens Send", async () => {
  const app = await open("../nsh.json");
  await app.waitFor("text:Nearby");
  await app.press("role:button[Send]");
  // Phones say "Files", computers "Choose files".
  await app.waitFor("text:/^(Files|Choose files)$/");
  expect(app.texts()).toContain("Send");
  await app.back();
  await app.keyboard.press("Control+3");
  await app.waitFor("text:Privacy");
  expect(app.texts()).toContain("Visible to nearby devices");
});
