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
    expect(() => validateService(route("https://my-nas.fnos.net/"), "my-nas")).not.toThrow();
  });
  it("rejects SSRF, foreign NAS, credentials and query secrets", () => {
    for (const url of [
      "https://127.0.0.1/",
      "https://other-nas.fnos.net/",
      "https://my-nas.fnos.net.evil.test/",
      "https://my-nas.fnos.net/?token=secret",
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
  it("enforces integer ports and supports the full service port range", () => {
    for (const port of [22, 80, 443, 8084, 65535])
      expect(() => validateService({ ...route(), localPort: port }, "my-nas")).not.toThrow();
    for (const port of [0, 65536, 1234.5])
      expect(() => validateService({ ...route(), localPort: port }, "my-nas")).toThrow();
  });
  it("prefers the service port and avoids occupied ports", () => {
    expect(suggestedLocalPort(8084)).toBe(8084);
    expect(suggestedLocalPort(8084, [8084, 8085])).toBe(8086);
    expect(suggestedLocalPort(65000)).toBe(65000);
    expect(suggestedLocalPort(80)).toBe(80);
    expect(suggestedLocalPort(65535, [65535, 1])).toBe(2);
    for (const port of [0, 65536, 1.5, NaN]) expect(() => suggestedLocalPort(port)).toThrow();
    expect(() =>
      suggestedLocalPort(
        8084,
        Array.from({ length: 65535 }, (_, i) => i + 1),
      ),
    ).toThrow();
    expect(localUrl(18084)).toBe("http://127.0.0.1:18084/");
  });
});
