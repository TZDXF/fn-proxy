import { i18n } from "./i18n";

/** Locale-neutral text produced by the Rust backend; translated here via i18n. */
export interface BackendText {
  code: string;
  params?: Record<string, string>;
}

export function isBackendText(value: unknown): value is BackendText {
  return (
    typeof value === "object" && value !== null && typeof (value as BackendText).code === "string"
  );
}

/** Resolves a backend text code in the active locale; plain strings pass through. */
export function localize(value: string | BackendText | null | undefined): string {
  if (value === null || value === undefined) return "";
  if (typeof value === "string") return value;
  if (!value.code) return "";
  const t = i18n.global;
  const scoped = `backend.${value.code}`;
  if (t.te(scoped)) return t.t(scoped, value.params ?? {});
  // Allows backend texts to reuse existing top-level keys such as docker.source.
  if (t.te(value.code)) return t.t(value.code, value.params ?? {});
  return value.code;
}

/** Normalizes Tauri command rejections: structured backend texts, Errors or strings. */
export function localizeError(error: unknown): string {
  if (isBackendText(error)) return localize(error);
  if (error instanceof Error) return error.message;
  return String(error ?? "");
}
