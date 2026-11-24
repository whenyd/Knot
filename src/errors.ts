/** Turns anything a command threw into one localized sentence. */
import type { BackendMessage } from "./api";
import { t } from "./i18n";
import { translateBackendMessage } from "./i18n/backend";

function isBackendMessage(value: unknown): value is BackendMessage {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as { key?: unknown }).key === "string"
  );
}

export function describe(error: unknown): string {
  if (isBackendMessage(error)) return translateBackendMessage(error);
  if (typeof error === "string") return t("error.unexpected", { detail: error });
  if (error instanceof Error) return t("error.unexpected", { detail: error.message });
  return t("error.unknown");
}
