// Comma-separated text, as typed or pasted into a TagInput.
export function parseList(text: string): string[] {
  return text
    .split(',')
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}
