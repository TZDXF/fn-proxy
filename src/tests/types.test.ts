import { describe, expect, it } from "vitest";
import {
  localUrl,
  normalizeFnId,
  suggestedLocalPort,
  validateService,
  type ServiceRoute,
} from "../lib/types";
const route = (upstream = "https://hash-0.my-nas.fnos.net/"): ServiceRoute => ({
  id: "1",
  name: "API",
  nasPort: 8084,
  localPort: 18084,
  upstream,
  enabled: true,
});
describe("profile and route validation", () => {
  it("normalizes only valid FN IDs", () => {
    expect(normalizeFnId(" My-NAS ")).toBe("my-nas");
    for (const id of ["", "https://my-nas.fnos.net", "../nas", "nas--id", "-nas"])
      expect(() => normalizeFnId(id)).toThrow();
  });
  it("accepts this NAS's HTTPS service root", () => {
    expect(() => validateService(route(), "my-nas")).not.toThrow();
  });
  it("rejects SSRF, foreign NAS, credentials and query secrets", () => {
    for (const url of [
      "https://127.0.0.1/",
      "https://hash.other-nas.fnos.net/",
      "https://hash.my-nas.fnos.net.evil.test/",
      "http://hash.my-nas.fnos.net/",
      "https://user:pass@hash.my-nas.fnos.net/",
      "https://hash.my-nas.fnos.net/?token=secret",
      "https://hash.my-nas.fnos.net/api",
      "https://hash.my-nas.fnos.net:8443/",
    ])
      expect(() => validateService(route(url), "my-nas")).toThrow();
  });
  it("enforces integer ports and avoids privileged listeners", () => {
    for (const port of [0, 22, 80, 65536, 1234.5])
      expect(() => validateService({ ...route(), localPort: port }, "my-nas")).toThrow();
  });
  it("suggests independent local ports", () => {
    expect(suggestedLocalPort(8084)).toBe(18084);
    expect(suggestedLocalPort(8084, [18084, 18085])).toBe(18086);
    expect(suggestedLocalPort(65000)).toBe(18080);
    expect(localUrl(18084)).toBe("http://127.0.0.1:18084/");
  });
});
