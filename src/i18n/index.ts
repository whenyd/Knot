/**
 * Runtime language state and formatting helpers.
 *
 * The app has exactly two UI languages. The selected mode is either a
 * concrete language or `system`; `system` is resolved once at startup.
 */
import { en } from "./messages.en";
import { zh, type MessageKey, type PluralBase } from "./messages.zh";

export type { MessageKey, PluralBase } from "./messages.zh";

export type Language = "zh" | "en";
export type LanguageMode = Language | "system";

export const LANGUAGE_STORAGE_KEY = "knot.language.v1";
export const LOCALES: Record<Language, string> = { zh: "zh-CN", en: "en-US" };

const dictionaries: Record<Language, Record<MessageKey, string>> = { zh, en };

export function detectSystemLanguage(): Language {
  const tag = typeof navigator === "undefined" ? "" : (navigator.language ?? "");
  return tag.toLowerCase().startsWith("zh") ? "zh" : "en";
}

let language: Language = detectSystemLanguage();

export function getLanguage(): Language {
  return language;
}

export function readLanguageMode(): LanguageMode {
  try {
    if (typeof localStorage === "undefined") return "system";
    const stored = localStorage.getItem(LANGUAGE_STORAGE_KEY);
    return stored === "zh" || stored === "en" || stored === "system" ? stored : "system";
  } catch {
    return "system";
  }
}

export function resolveLanguage(mode: LanguageMode): Language {
  return mode === "system" ? detectSystemLanguage() : mode;
}

function applyDocumentLanguage(): void {
  if (typeof document === "undefined") return;
  document.documentElement.lang = LOCALES[language];
  document.documentElement.dataset.language = language;
}

/** Resolve the saved mode and apply it to `<html lang>`. */
export function initI18n(): Language {
  language = resolveLanguage(readLanguageMode());
  applyDocumentLanguage();
  return language;
}

/**
 * Persist and apply a mode. Returns whether the resolved language changed,
 * so callers can skip a full re-render when nothing changed.
 */
export function setLanguageMode(mode: LanguageMode): boolean {
  try {
    localStorage.setItem(LANGUAGE_STORAGE_KEY, mode);
  } catch {
    // Storage failures only affect the next launch.
  }
  const next = resolveLanguage(mode);
  const changed = next !== language;
  language = next;
  applyDocumentLanguage();
  return changed;
}

/** Test-only switch that avoids touching localStorage or the DOM. */
export function setLanguageForTest(next: Language): void {
  language = next;
}

export type MessageParams = Record<string, string | number>;

/** Replace `{name}` placeholders. Values are never interpolated again. */
export function interpolate(template: string, params?: MessageParams): string {
  return template.replace(/\{(\w+)\}/g, (_, name: string) => {
    const value = params?.[name];
    return value === undefined ? `{${name}}` : String(value);
  });
}

export function t(key: MessageKey, params?: MessageParams): string {
  return interpolate(dictionaries[language][key], params);
}

/** Look up a pluralized key (`base.one` / `base.other`). */
export function tp(base: PluralBase, count: number, params?: MessageParams): string {
  const category = new Intl.PluralRules(LOCALES[language]).select(count);
  const suffix = category === "one" ? "one" : "other";
  return t(`${base}.${suffix}` as MessageKey, { count, ...params });
}

/** Whether a key exists in the active dictionary. Used by static HTML binding. */
export function hasMessage(key: string): key is MessageKey {
  return Object.prototype.hasOwnProperty.call(dictionaries[language], key);
}
