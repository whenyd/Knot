/**
 * Display-only date formatting.
 *
 * Stored values stay `YYYY-MM-DDTHH:MM` local text; sorting compares that
 * raw string, so nothing here may change the persisted format.
 */
import { LOCALES, t } from "./index";

const SCHEDULE_RE = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})$/;

// Both UI languages use the same numeric format: 2026/11/04 09:00.
const SCHEDULE_FORMATTER = new Intl.DateTimeFormat(LOCALES.zh, {
  year: "numeric",
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
  hourCycle: "h23",
});

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

  return SCHEDULE_FORMATTER.format(date);
}
