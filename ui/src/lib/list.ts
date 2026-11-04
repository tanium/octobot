// Comma-separated text fields for string lists, like muted repos.

export function parseList(text: string): string[] {
  return text
    .split(',')
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

export function formatList(items: string[]): string {
  return items.join(', ');
}
