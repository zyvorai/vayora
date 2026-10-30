"use client";
import { useCallback, useEffect, useState } from "react";
import { LANGS, pickLang, translate, type Key, type Lang } from "@/lib/i18n";

const STORE = "zyvor:lang";

/** The payer's language: `?lang=`, then their last choice, then the browser's. Starts in English so the server and first paint agree. */
export function useI18n() {
  const [lang, setLangState] = useState<Lang>("en");
  useEffect(() => {
    let saved: string | null = null;
    try {
      saved = localStorage.getItem(STORE);
    } catch {}
    const l = pickLang(new URLSearchParams(location.search).get("lang"), saved, navigator.languages ?? [navigator.language]);
    setLangState(l);
    document.documentElement.lang = l;
  }, []);
  const setLang = useCallback((l: Lang) => {
    setLangState(l);
    document.documentElement.lang = l;
    try {
      localStorage.setItem(STORE, l);
    } catch {}
  }, []);
  const t = useCallback((key: Key, vars?: Record<string, string>) => translate(lang, key, vars), [lang]);
  return { lang, setLang, t };
}

export function LangPicker({ lang, setLang, label }: { lang: Lang; setLang: (l: Lang) => void; label: string }) {
  return (
    <label className="langpick">
      <span className="sr">{label}</span>
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" aria-hidden="true" style={{ verticalAlign: "-2px", marginRight: 2, color: "var(--muted)" }}><circle cx="12" cy="12" r="9" /><path d="M3 12h18M12 3c3 3.2 3 14.8 0 18M12 3c-3 3.2-3 14.8 0 18" /></svg>
      <select value={lang} onChange={(e) => setLang(e.target.value as Lang)} aria-label={label}>
        {(Object.keys(LANGS) as Lang[]).map((l) => <option key={l} value={l}>{LANGS[l]}</option>)}
      </select>
    </label>
  );
}
