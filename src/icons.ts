// Linien-Icons als Inline-SVG (24er-Viewbox, Strich in `currentColor`), damit
// sie Textfarbe, Hover und Zustandsfarben des Knopfs erben. Die Icons sind
// dekorativ (`aria-hidden`); Name und Tooltip traegt der umgebende Knopf.

const ICON_PATHS = {
  trash:
    '<path d="M4 7h16"/><path d="M9 7V4.5h6V7"/><path d="M6 7l1 12.5a1.5 1.5 0 0 0 1.5 1.5h7a1.5 1.5 0 0 0 1.5-1.5L18 7"/><path d="M10 11v6M14 11v6"/>',
  settings:
    '<path d="M10.23 4.61L10.28 2.15L13.72 2.15L13.77 4.61L15.97 5.52L17.75 3.82L20.18 6.25L18.48 8.03L19.39 10.23L21.85 10.28L21.85 13.72L19.39 13.77L18.48 15.97L20.18 17.75L17.75 20.18L15.97 18.48L13.77 19.39L13.72 21.85L10.28 21.85L10.23 19.39L8.03 18.48L6.25 20.18L3.82 17.75L5.52 15.97L4.61 13.77L2.15 13.72L2.15 10.28L4.61 10.23L5.52 8.03L3.82 6.25L6.25 3.82L8.03 5.52Z"/><circle cx="12" cy="12" r="3"/>',
  sync:
    '<path d="M19.5 10A8 8 0 0 0 5.6 7.2L4 9"/><path d="M4 4.5V9h4.5"/><path d="M4.5 14a8 8 0 0 0 13.9 2.8L20 15"/><path d="M20 19.5V15h-4.5"/>',
  warning: '<path d="M12 3.5 2.5 20h19z"/><path d="M12 10v4.5"/><path d="M12 17.5h.01"/>',
  check: '<path d="M5 12.5l4.5 4.5L19 7.5"/>',
  pause: '<path d="M9 5.5v13M15 5.5v13"/>',
  lock: '<rect x="5" y="11" width="14" height="10" rx="2"/><path d="M8 11V7.5a4 4 0 0 1 8 0V11"/>',
  pencil: '<path d="M4 20h4L19 9a2.83 2.83 0 0 0-4-4L4 16z"/><path d="M13.5 6.5l4 4"/>',
  close: '<path d="M6 6l12 12M18 6 6 18"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  dots: '<path d="M6 12h.01M12 12h.01M18 12h.01" stroke-width="3"/>',
} as const;

export type IconName = keyof typeof ICON_PATHS;

export function icon(name: IconName): string {
  return `<svg class="icon icon-${name}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">${ICON_PATHS[name]}</svg>`;
}
