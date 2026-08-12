function normalizedIso(value: string): string | null {
  const epoch = Date.parse(value);
  return Number.isFinite(epoch) ? new Date(epoch).toISOString() : null;
}

export function formatCompactTimestamp(value: string): string {
  const iso = normalizedIso(value);
  return iso === null ? value : iso.slice(0, 10);
}

export function formatDetailedTimestamp(value: string): string {
  const iso = normalizedIso(value);
  return iso === null ? value : `${iso.slice(0, 10)} ${iso.slice(11, 16)} UTC`;
}
