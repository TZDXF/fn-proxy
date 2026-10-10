import { describe, expect, it } from "vitest";
import { formatExpiry, formatTrafficMb, trafficPercent } from "../lib/fn-connect";
import type { FnConnectEntitlement } from "../lib/types";
const plan: FnConnectEntitlement = {
  tier: "premium",
  bandwidthMbps: 12,
  trafficUsedMb: 1024,
  trafficPerMonthMb: 512000,
  endTime: 1936742400,
};
describe("FN Connect display", () => {
  it("uses the NAS's MB/1024 units rather than byte or decimal conversions", () => {
    expect(formatTrafficMb(1024, "zh-CN")).toBe("1 GB");
    expect(formatTrafficMb(512000, "en-US")).toBe("500 GB");
    expect(formatTrafficMb(0, "en-US")).toBe("0 MB");
    expect(formatTrafficMb(512, "en-US")).toBe("512 MB");
    expect(formatTrafficMb(1536, "en-US")).toBe("1.5 GB");
  });
  it("never turns missing or invalid data into zero usage", () => {
    for (const n of [null, -1, Infinity, NaN]) expect(formatTrafficMb(n, "en-US")).toBe("—");
    expect(trafficPercent({ ...plan, trafficUsedMb: null })).toBeNull();
    expect(trafficPercent({ ...plan, trafficPerMonthMb: null })).toBeNull();
  });
  it("clamps exhausted traffic and omits progress for unlimited traffic", () => {
    expect(trafficPercent(plan)).toBeCloseTo(0.2);
    expect(trafficPercent({ ...plan, trafficUsedMb: 600000 })).toBe(100);
    expect(trafficPercent({ ...plan, trafficUsedMb: 0 })).toBe(0);
    expect(trafficPercent({ ...plan, trafficPerMonthMb: 0 })).toBeNull();
    expect(trafficPercent({ ...plan, trafficUsedMb: Infinity })).toBeNull();
  });
  it("converts seconds, handles absent dates, and formats both locales", () => {
    expect(formatExpiry(Date.UTC(2031, 4, 17, 12) / 1000, "zh-CN")).toContain("2031");
    expect(formatExpiry(Date.UTC(2031, 4, 17, 12) / 1000, "en-US")).toContain("2031");
    for (const n of [null, 0, -1, NaN, 1e20]) expect(formatExpiry(n, "en-US")).toBeNull();
  });
});
import { createSSRApp } from "vue";
import { renderToString } from "vue/server-renderer";
import { createI18n } from "vue-i18n";
import FnConnectCard from "../components/FnConnectCard.vue";
import zhCN from "../locales/zh-CN.json";
import enUS from "../locales/en-US.json";
import type { ConnectionInfo } from "../lib/types";

async function renderCard(info: ConnectionInfo, locale = "zh-CN") {
  const app = createSSRApp(FnConnectCard, { connection: info });
  app.use(createI18n({ legacy: false, locale, messages: { "zh-CN": zhCN, "en-US": enUS } }));
  return renderToString(app);
}
const connected: ConnectionInfo = {
  connected: true,
  fnId: "fixture-nas",
  username: "fixture-user",
  relay: "https://fixture-nas.fnos.net",
  authMode: "ticket-cookie",
  message: { code: "auth.loggedIn" },
  fnConnect: { status: "available", entitlement: plan },
};
describe("FN Connect inline rendering", () => {
  it("shows premium bandwidth and usage inline, with expiry in the tooltip", async () => {
    const html = await renderCard(connected);
    for (const text of ["FN Connect", "Premium", "12 Mbps", "1 GB / 500 GB", "截至"])
      expect(html).toContain(text);
    expect(html).not.toContain("fixture-user");
    expect(html).toContain('class="fn-connect-inline"');
    expect(html).toContain('title="FN Connect · Premium');
    for (const tag of ["<section", "<dl", "<br", 'role="progressbar"'])
      expect(html).not.toContain(tag);
  });
  it("shows Base and unlimited traffic without a fake expiry or progress bar", async () => {
    const html = await renderCard({
      ...connected,
      fnConnect: {
        status: "available",
        entitlement: { ...plan, tier: "base", trafficPerMonthMb: 0 },
      },
    });
    expect(html).toContain("Base");
    expect(html).toContain("不限流量");
    expect(html).not.toContain("截至");
    expect(html).not.toContain('role="progressbar"');
  });
  it("shows Pro and localizes labels in English", async () => {
    const html = await renderCard(
      { ...connected, fnConnect: { status: "available", entitlement: { ...plan, tier: "pro" } } },
      "en-US",
    );
    for (const text of ["Pro", "Bandwidth", "Monthly high-speed traffic", "Until"])
      expect(html).toContain(text);
  });
  it("does not mislabel unknown tiers, missing info or disconnected cached data", async () => {
    const unknown = await renderCard({
      ...connected,
      fnConnect: {
        status: "available",
        entitlement: { ...plan, tier: "unknown", bandwidthMbps: null },
      },
    });
    expect(unknown).toContain("未知等级");
    expect(unknown).not.toContain("Base");
    expect(await renderCard({ ...connected, fnConnect: null })).toContain("正在读取套餐信息");
    expect(
      await renderCard({ ...connected, fnConnect: { status: "unbound", entitlement: null } }),
    ).toContain("未绑定飞牛账号");
    expect(
      await renderCard({ ...connected, fnConnect: { status: "unavailable", entitlement: null } }),
    ).toContain("套餐信息暂不可用");
    const disconnected = await renderCard({ ...connected, connected: false });
    expect(disconnected).toContain("连接后显示套餐与流量");
    expect(disconnected).not.toContain("500 GB");
  });
});

// Ensure each saved connection stays in exactly one existing list/table row.
import { readFileSync } from "node:fs";
it("integrates metadata into existing rows without an extra card or table row", () => {
  const source = readFileSync(new URL("../App.vue", import.meta.url), "utf8");
  expect(source).toContain('class="overview-row"');
  expect(source).toContain('<tr v-for="c in savedConnections"');
  expect(source).toContain('<td><FnConnectCard :connection="c.connection" /></td>');
  expect(source).not.toContain("fn-connect-table-row");
  expect(source).not.toContain("overview-connection");
});
