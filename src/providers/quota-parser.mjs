export function parseResetAtFromText(text, now = Date.now()) {
  if (!text) return null;
  const normalized = String(text).replace(/\s+/g, ' ').trim();

  const relative = normalized.match(/(?:try again|reset(?:s)?|available again)\s+in\s+(?:(\d+)\s*(?:h|hour|hours))?\s*(?:(\d+)\s*(?:m|min|minute|minutes))?\s*(?:(\d+)\s*(?:s|sec|second|seconds))?/i);
  if (relative && (relative[1] || relative[2] || relative[3])) {
    const ms = (Number(relative[1] ?? 0) * 3600 + Number(relative[2] ?? 0) * 60 + Number(relative[3] ?? 0)) * 1000;
    return new Date(now + ms).toISOString();
  }

  const iso = normalized.match(/\b(20\d{2}-\d{2}-\d{2}[T ][0-2]\d:[0-5]\d(?::[0-5]\d)?(?:Z|[+-]\d{2}:?\d{2})?)\b/);
  if (iso) {
    const parsed = Date.parse(iso[1]);
    if (!Number.isNaN(parsed)) return new Date(parsed).toISOString();
  }

  const phrase = normalized.match(/(?:reset(?:s)?|try again|available again)\s+(?:at|on)\s+([^.;]+)/i);
  if (phrase) {
    const parsed = Date.parse(phrase[1]);
    if (!Number.isNaN(parsed) && parsed > now) return new Date(parsed).toISOString();
  }
  return null;
}
