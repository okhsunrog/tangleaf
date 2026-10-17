import c from "@shikijs/langs/c";
import css from "@shikijs/langs/css";
import diff from "@shikijs/langs/diff";
import html from "@shikijs/langs/html";
import javascript from "@shikijs/langs/javascript";
import json from "@shikijs/langs/json";
import markdown from "@shikijs/langs/markdown";
import python from "@shikijs/langs/python";
import rust from "@shikijs/langs/rust";
import shellscript from "@shikijs/langs/shellscript";
import sql from "@shikijs/langs/sql";
import toml from "@shikijs/langs/toml";
import typescript from "@shikijs/langs/typescript";
import yaml from "@shikijs/langs/yaml";
import githubDark from "@shikijs/themes/github-dark";
import githubLight from "@shikijs/themes/github-light";
import githubLightHighContrast from "@shikijs/themes/github-light-high-contrast";
import { createHighlighterCoreSync, type ThemeRegistrationRaw } from "shiki/core";
import { createJavaScriptRegexEngine } from "shiki/engine/javascript";

import {
  canHighlightMarkdownCode,
  normalizeLanguageLabel,
  plainMarkdownCode,
  resolveMarkdownCodeLanguage,
  type CodeTheme,
  type MarkdownCodeHighlight,
} from "./syntax-highlighter";

/**
 * E-ink with a grayscale panel has no hue to spend, so the mono theme carries meaning with
 * weight and slant only. @shikijs/themes ships no monochrome theme, hence this minimal one.
 */
const EINK_MONO_THEME: ThemeRegistrationRaw = {
  name: "eink-mono",
  type: "light",
  colors: { "editor.background": "#ffffff", "editor.foreground": "#000000" },
  settings: [
    { settings: { background: "#ffffff", foreground: "#000000" } },
    {
      scope: ["comment", "punctuation.definition.comment"],
      settings: { foreground: "#555555" },
    },
    {
      scope: ["keyword", "storage", "storage.type", "constant.language"],
      settings: { fontStyle: "bold" },
    },
    { scope: ["string", "string.quoted"], settings: { fontStyle: "italic" } },
  ],
};

/** The e-ink profile is light-only, so both variants resolve to the same theme there. */
function themeNames(theme: CodeTheme): { light: string; dark: string } {
  if (theme === "eink") {
    return { light: "github-light-high-contrast", dark: "github-light-high-contrast" };
  }
  if (theme === "eink-mono") return { light: "eink-mono", dark: "eink-mono" };
  return { light: "github-light", dark: "github-dark" };
}

const MAX_HIGHLIGHT_LINE_CHARS = 4_096;
const MAX_CACHEABLE_CHARS = 8_192;
const MAX_CACHE_ENTRIES = 24;
const TOKENIZE_TIME_LIMIT_MS = 5;
const FONT_STYLE_ITALIC = 1;
const FONT_STYLE_BOLD = 2;
type MarkdownHighlighter = ReturnType<typeof createHighlighterCoreSync>;
let highlighter: MarkdownHighlighter | null = null;

const highlightCache = new Map<string, MarkdownCodeHighlight>();

export function highlightMarkdownCode(
  code: string,
  requestedLanguage: string | null,
  theme: CodeTheme = "standard",
): MarkdownCodeHighlight {
  const cleanRequest = normalizeLanguageLabel(requestedLanguage);
  const language = resolveMarkdownCodeLanguage(cleanRequest);
  const highlighted = canHighlightMarkdownCode(code, cleanRequest);
  const cacheKey = `${cleanRequest}\u0000${theme}\u0000${highlighted ? "highlight" : "plain"}\u0000${code}`;
  const cached = code.length <= MAX_CACHEABLE_CHARS ? highlightCache.get(cacheKey) : undefined;
  if (cached) return cached;

  let result: MarkdownCodeHighlight;
  if (!highlighted) {
    result = {
      highlighted: false,
      language: "text",
      lines: plainMarkdownCode(code, cleanRequest).lines,
      requestedLanguage: cleanRequest,
    };
  } else {
    try {
      const mono = theme === "eink-mono";
      const lines = getHighlighter().codeToTokensWithThemes(code, {
        lang: language,
        themes: themeNames(theme),
        tokenizeMaxLineLength: MAX_HIGHLIGHT_LINE_CHARS,
        tokenizeTimeLimit: TOKENIZE_TIME_LIMIT_MS,
      });
      result = {
        highlighted: true,
        language,
        lines: lines.map((line) =>
          line.map((token) => {
            // Weight and slant are only read from the mono theme; the color themes keep the
            // plain rendering they have always had.
            const style = mono ? (token.variants.light?.fontStyle ?? 0) : 0;
            return {
              content: token.content,
              darkColor: token.variants.dark?.color,
              lightColor: token.variants.light?.color,
              bold: (style & FONT_STYLE_BOLD) !== 0 || undefined,
              italic: (style & FONT_STYLE_ITALIC) !== 0 || undefined,
            };
          }),
        ),
        requestedLanguage: cleanRequest,
      };
    } catch {
      result = {
        highlighted: false,
        language: "text",
        lines: plainMarkdownCode(code, cleanRequest).lines,
        requestedLanguage: cleanRequest,
      };
    }
  }

  if (code.length <= MAX_CACHEABLE_CHARS) {
    highlightCache.set(cacheKey, result);
    if (highlightCache.size > MAX_CACHE_ENTRIES) {
      const oldest = highlightCache.keys().next().value;
      if (oldest !== undefined) highlightCache.delete(oldest);
    }
  }
  return result;
}

function getHighlighter(): MarkdownHighlighter {
  highlighter ??= createHighlighterCoreSync({
    engine: createJavaScriptRegexEngine({ target: "ES2018" }),
    langs: [
      c,
      css,
      diff,
      html,
      javascript,
      json,
      markdown,
      python,
      rust,
      shellscript,
      sql,
      toml,
      typescript,
      yaml,
    ],
    themes: [githubLight, githubDark, githubLightHighContrast, EINK_MONO_THEME],
  });
  return highlighter;
}
