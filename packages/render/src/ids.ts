const ALPHABET = "0123456789abcdefghjkmnpqrstvwxyz";

/** A new id: a type letter, a hyphen and four random characters. */
export function randomId(letter: string): string {
  const values = new Uint8Array(4);
  crypto.getRandomValues(values);
  let out = letter + "-";
  for (const value of values) out += ALPHABET[value & 31];
  return out;
}
