import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("vue", () => ({ createApp: () => ({ mount: vi.fn() }) }));
vi.mock("../App.vue", () => ({ default: {} }));
vi.mock("../style.css", () => ({}));

afterEach(() => {
  vi.unstubAllGlobals();
  vi.unstubAllEnvs();
  vi.resetModules();
});

async function loadApp(production: boolean) {
  const document = new EventTarget();
  vi.stubGlobal("document", document);
  vi.stubEnv("PROD", production);
  await import("../main");
  return document;
}

describe("release context menu", () => {
  it("prevents the default context menu in production", async () => {
    const document = await loadApp(true);
    const event = new Event("contextmenu", { cancelable: true });
    document.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
  });

  it("keeps the default context menu in development", async () => {
    const document = await loadApp(false);
    const event = new Event("contextmenu", { cancelable: true });
    document.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
  });

  it("does not prevent normal clicks in production", async () => {
    const document = await loadApp(true);
    const event = new Event("click", { cancelable: true });
    document.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
  });
});
