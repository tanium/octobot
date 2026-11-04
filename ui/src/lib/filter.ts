// Case-insensitive substring match against any of the given fields.
export function matches(needle: string, ...fields: string[]): boolean {
  const n = needle.trim().toLowerCase();
  return n === '' || fields.some((f) => f.toLowerCase().includes(n));
}
