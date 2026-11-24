/**
 * Display-only date formatting.
 *
 * Stored values stay `YYYY-MM-DDTHH:MM` local text; sorting compares that
 * raw string, so nothing here may change the persisted format.
 */
import { getLanguage, LOCALES, t, type Language } from "./index";

const SCHEDULE_RE = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})$/;

const OPTIONS: Record<Language, Intl.DateTimeFormatOptions> = {
  zh: {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  },
  en: {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  },
};

const formatters = new Map<Language, Intl.DateTimeFormat>();

function formatter(): Intl.DateTimeFormat {
  const language = getLanguage();
  let value = formatters.get(language);
  if (!value) {
    value = new Intl.DateTimeFormat(LOCALES[language], OPTIONS[language]);
    formatters.set(language, value);
  }
  return value;
}

/** Format a stored local timestamp for display; unparseable input stays readable. */
export function formatSchedule(value: string | null): string {
  if (value === null) return t("common.dash");
  const match = SCHEDULE_RE.exec(value);
  if (!match) return value.replace("T", " ");

  const [year, month, day, hour, minute] = match.slice(1).map(Number);
  const date = new Date(year, month - 1, day, hour, minute);

  // Reject values that Date silently normalized (e.g. month 13).
  if (
    date.getFullYear() !== year ||
    date.getMonth() !== month - 1 ||
    date.getDate() !== day ||
    date.getHours() !== hour ||
    date.getMinutes() !== minute
  ) {
    return value.replace("T", " ");
  }

  return formatter().format(date);
}
