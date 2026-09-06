import type { Collection, VideoStatusFilter } from "./types";

export function coerceBool(value: unknown, fallback: boolean): boolean {
  return typeof value === "boolean" ? value : fallback;
}

export function formatDate(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleDateString(undefined, { year: "numeric", month: "long", day: "numeric" });
}

export function normalizeSearch(value: string): string {
  return value.toLocaleLowerCase().normalize("NFKD").replace(/[\u0300-\u036f]/g, "");
}

export function isVideoStatusFilter(value: string | undefined): value is VideoStatusFilter {
  return (
    value === "all" ||
    value === "transcript" ||
    value === "missing-transcript" ||
    value === "summary" ||
    value === "missing-summary"
  );
}

export function compareCollections(a: Collection, b: Collection): number {
  return a.name.localeCompare(b.name, undefined, { sensitivity: "base" });
}

export function capitalize(value: string): string {
  return value.charAt(0).toUpperCase() + value.slice(1);
}
