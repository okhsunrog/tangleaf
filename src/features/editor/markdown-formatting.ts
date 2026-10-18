import { EditorSelection, StateField, type Extension } from "@codemirror/state";
import { EditorView, keymap, showTooltip, type Tooltip } from "@codemirror/view";

export type MarkdownFormat = "bold" | "italic" | "code" | "link";

/** Selection stays inside the syntax, so a second invocation toggles it off. */
export function formatMarkdown(view: EditorView, format: MarkdownFormat): boolean {
  if (view.state.readOnly) return false;
  const { from, to } = view.state.selection.main;
  const text = view.state.sliceDoc(from, to);
  if (format === "link") {
    const label = text || "link text";
    const insert = `[${label}](https://)`;
    view.dispatch({
      changes: { from, to, insert },
      selection: EditorSelection.range(from + label.length + 3, from + insert.length - 1),
      userEvent: "input.format",
    });
  } else {
    const marker = format === "bold" ? "**" : format === "italic" ? "*" : "`";
    const surrounding =
      from >= marker.length &&
      view.state.sliceDoc(from - marker.length, from) === marker &&
      view.state.sliceDoc(to, to + marker.length) === marker;
    if (surrounding) {
      view.dispatch({
        changes: [
          { from: from - marker.length, to: from },
          { from: to, to: to + marker.length },
        ],
        selection: EditorSelection.range(from - marker.length, to - marker.length),
        userEvent: "input.format",
      });
    } else {
      view.dispatch({
        changes: { from, to, insert: `${marker}${text}${marker}` },
        selection: EditorSelection.range(from + marker.length, to + marker.length),
        userEvent: "input.format",
      });
    }
  }
  view.focus();
  return true;
}

const actions = [
  { format: "bold", label: "Bold", text: "B", key: "Mod-b", shortcut: "Ctrl/⌘ B" },
  { format: "italic", label: "Italic", text: "I", key: "Mod-i", shortcut: "Ctrl/⌘ I" },
  {
    format: "code",
    label: "Inline code",
    text: "</>",
    key: "Mod-Shift-c",
    shortcut: "Ctrl/⌘ Shift C",
  },
  {
    format: "link",
    label: "Insert link",
    text: "Link",
    key: "Mod-Shift-k",
    shortcut: "Ctrl/⌘ Shift K",
  },
] as const;

const formattingTooltip = StateField.define<readonly Tooltip[]>({
  create: () => [],
  update(_previous, transaction) {
    const { main } = transaction.state.selection;
    if (main.empty || transaction.state.readOnly) return [];
    return [
      {
        pos: main.from,
        above: true,
        strictSide: false,
        create(view) {
          const dom = document.createElement("div");
          dom.className = "markdown-formatting";
          dom.setAttribute("role", "toolbar");
          dom.setAttribute("aria-label", "Format selection");
          for (const action of actions) {
            const button = document.createElement("button");
            button.type = "button";
            button.textContent = action.text;
            button.title = `${action.label} (${action.shortcut})`;
            button.setAttribute("aria-label", action.label);
            button.addEventListener("mousedown", (event) => event.preventDefault());
            button.addEventListener("click", () => formatMarkdown(view, action.format));
            dom.append(button);
          }
          return { dom };
        },
      },
    ];
  },
  provide: (field) => showTooltip.computeN([field], (state) => state.field(field)),
});

export function markdownFormatting(): Extension {
  return [
    keymap.of(
      actions.map((action) => ({
        key: action.key,
        run: (view: EditorView) => formatMarkdown(view, action.format),
      })),
    ),
    formattingTooltip,
    EditorView.baseTheme({
      ".cm-tooltip:has(.markdown-formatting)": {
        background: "var(--popover)",
        color: "var(--popover-foreground)",
        border: "1px solid var(--border)",
        borderRadius: "8px",
        boxShadow: "0 4px 16px #0002",
      },
      ".markdown-formatting": { display: "flex", gap: "2px", padding: "4px" },
      ".markdown-formatting button": {
        minWidth: "32px",
        height: "30px",
        padding: "0 8px",
        borderRadius: "4px",
        cursor: "pointer",
      },
      ".markdown-formatting button:hover, .markdown-formatting button:focus-visible": {
        background: "var(--accent)",
        outline: "2px solid var(--primary)",
      },
      '.markdown-formatting button[aria-label="Bold"]': { fontWeight: "700" },
      '.markdown-formatting button[aria-label="Italic"]': { fontStyle: "italic" },
    }),
  ];
}
