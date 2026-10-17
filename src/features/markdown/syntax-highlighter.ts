/** Which Shiki theme pair a code block is tokenized with. */
export type CodeTheme = "standard" | "eink" | "eink-mono";

export function resolveCodeTheme(
  display: "standard" | "eink",
  inkColor: "color" | "mono",
): CodeTheme {
  if (display !== "eink") return "standard";
  return inkColor === "mono" ? "eink-mono" : "eink";
}

const MAX_HIGHLIGHT_CHARS = 32_000;
const MAX_HIGHLIGHT_LINES = 200;
const MAX_HIGHLIGHT_LINE_CHARS = 4_096;

const LANGUAGE_ALIASES: Readonly<Record<string, string>> = {
  bash: "shellscript",
  c: "c",
  console: "shellscript",
  css: "css",
  diff: "diff",
  html: "html",
  javascript: "javascript",
  js: "javascript",
  json: "json",
  jsonc: "json",
  jsx: "javascript",
  markdown: "markdown",
  md: "markdown",
  patch: "diff",
  plain: "text",
  plaintext: "text",
  py: "python",
  python: "python",
  rs: "rust",
  rust: "rust",
  sh: "shellscript",
  shell: "shellscript",
  shellscript: "shellscript",
  sql: "sql",
  text: "text",
  toml: "toml",
  ts: "typescript",
  tsx: "typescript",
  txt: "text",
  typescript: "typescript",
  yaml: "yaml",
  yml: "yaml",
  zsh: "shellscript",
};

export interface MarkdownCodeToken {
  content: string;
  darkColor?: string;
  lightColor?: string;
  /** Only carried by the monochrome theme, which has nothing but weight and slant to spend. */
  bold?: boolean;
  italic?: boolean;
}

export interface MarkdownCodeHighlight {
  highlighted: boolean;
  language: string;
  lines: MarkdownCodeToken[][];
  requestedLanguage: string | null;
}

export type SyntaxHighlighterRuntime = typeof import("./syntax-highlighter-runtime");
let runtime: SyntaxHighlighterRuntime | null = null;
let loading: Promise<SyntaxHighlighterRuntime> | null = null;
const listeners = new Set<() => void>();

export function cachedSyntaxHighlighter(): SyntaxHighlighterRuntime | null {
  return runtime;
}

export function subscribeSyntaxHighlighter(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function loadSyntaxHighlighter(): Promise<SyntaxHighlighterRuntime> {
  if (runtime) return Promise.resolve(runtime);
  loading ??= import("./syntax-highlighter-runtime").then(
    (loaded) => {
      runtime = loaded;
      for (const listener of listeners) listener();
      return loaded;
    },
    (error: unknown) => {
      // A later eligible block or edit may retry; keep the current code readable on failure.
      loading = null;
      throw error;
    },
  );
  return loading;
}

export function resolveMarkdownCodeLanguage(requestedLanguage: string | null): string {
  const label = normalizeLanguageLabel(requestedLanguage);
  return label && Object.prototype.hasOwnProperty.call(LANGUAGE_ALIASES, label)
    ? LANGUAGE_ALIASES[label]
    : "text";
}

export function canHighlightMarkdownCode(code: string, requestedLanguage: string | null): boolean {
  return resolveMarkdownCodeLanguage(requestedLanguage) !== "text" && isHighlightableSource(code);
}

export function plainMarkdownCode(
  code: string,
  requestedLanguage: string | null,
): MarkdownCodeHighlight {
  return {
    highlighted: false,
    language: "text",
    // Match the eventual line wrappers only for bounded, highlightable code.
    lines: canHighlightMarkdownCode(code, requestedLanguage)
      ? code.split("\n").map((content) => [{ content }])
      : plainLines(code),
    requestedLanguage: normalizeLanguageLabel(requestedLanguage),
  };
}

export function highlightMarkdownCode(
  code: string,
  requestedLanguage: string | null,
  theme: CodeTheme = "standard",
): MarkdownCodeHighlight {
  return (
    runtime?.highlightMarkdownCode(code, requestedLanguage, theme) ??
    plainMarkdownCode(code, requestedLanguage)
  );
}

export function extractMarkdownCodeLanguage(className: string | undefined): string | null {
  const match = /(?:^|\s)language-([^\s]+)/.exec(className ?? "");
  return normalizeLanguageLabel(match?.[1] ?? null);
}

export function normalizeLanguageLabel(value: string | null): string | null {
  if (!value) return null;
  const normalized = value.trim().toLowerCase().slice(0, 32);
  return /^[a-z0-9_+#.-]+$/.test(normalized) ? normalized : null;
}

function plainLines(code: string): MarkdownCodeToken[][] {
  return [[{ content: code }]];
}

function isHighlightableSource(code: string): boolean {
  if (code.length > MAX_HIGHLIGHT_CHARS) return false;
  let lineCount = 1;
  let lineLength = 0;
  for (let index = 0; index < code.length; index += 1) {
    if (code.charCodeAt(index) === 0x0a) {
      lineCount += 1;
      lineLength = 0;
      if (lineCount > MAX_HIGHLIGHT_LINES) return false;
    } else {
      lineLength += 1;
      if (lineLength > MAX_HIGHLIGHT_LINE_CHARS) return false;
    }
  }
  return true;
}
