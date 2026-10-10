import type { FnConnectEntitlement } from "./types";

export function formatTrafficMb(value: number | null, locale: string): string {
  if (value === null || !Number.isFinite(value) || value < 0) return "—";
  const gb = value >= 1024;
  return `${new Intl.NumberFormat(locale, { maximumFractionDigits: gb ? 2 : 1 }).format(
    gb ? value / 1024 : value,
  )} ${gb ? "GB" : "MB"}`;
}

export function trafficPercent(entitlement: FnConnectEntitlement): number | null {
  const { trafficUsedMb: used, trafficPerMonthMb: total } = entitlement;
  if (
    used === null ||
    total === null ||
    !Number.isFinite(used) ||
    !Number.isFinite(total) ||
    used < 0 ||
    total <= 0
  )
    return null;
  return Math.min(100, (used / total) * 100);
}

export function formatExpiry(seconds: number | null, locale: string): string | null {
  if (seconds === null || !Number.isFinite(seconds) || seconds <= 0) return null;
  const date = new Date(seconds * 1000);
  if (!Number.isFinite(date.getTime())) return null;
  const formatter = new Intl.DateTimeFormat(locale, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  });
  if (locale === "zh-CN") {
    const parts = formatter.formatToParts(date);
    return ["year", "month", "day"]
      .map((type) => parts.find((part) => part.type === type)?.value)
      .join(".");
  }
  return formatter.format(date);
}
