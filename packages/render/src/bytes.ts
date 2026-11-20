// Every `range` the core reports is a byte range in the UTF-8 body.
// JavaScript strings are indexed in UTF-16 code units, so convert first.

const encoder = new TextEncoder();
const decoder = new TextDecoder();

/** The string index that corresponds to a byte offset in the UTF-8 form of `text`. */
export function byteToIndex(text: string, byte: number): number {
  const bytes = encoder.encode(text);
  if (byte <= 0) return 0;
  if (byte >= bytes.length) return text.length;
  return decoder.decode(bytes.subarray(0, byte)).length;
}

/** The byte offset in the UTF-8 form of `text` that corresponds to a string index. */
export function indexToByte(text: string, index: number): number {
  return encoder.encode(text.slice(0, Math.max(0, index))).length;
}

/**
 * Converts many offsets for one text. Builds one table, then each lookup is
 * a binary search.
 */
export class ByteMap {
  /** `starts[i]` is the byte offset at which code unit `i` starts. */
  private readonly starts: Uint32Array;

  constructor(readonly text: string) {
    const starts = new Uint32Array(text.length + 1);
    let byte = 0;
    for (let i = 0; i < text.length; i++) {
      starts[i] = byte;
      const unit = text.charCodeAt(i);
      if (unit < 0x80) byte += 1;
      else if (unit < 0x800) byte += 2;
      else if (unit >= 0xd800 && unit <= 0xdbff && isLow(text.charCodeAt(i + 1))) {
        // A surrogate pair is one 4-byte sequence. Both halves start with it.
        starts[i + 1] = byte;
        byte += 4;
        i++;
      } else byte += 3;
    }
    starts[text.length] = byte;
    this.starts = starts;
  }

  get byteLength(): number {
    return this.starts[this.text.length]!;
  }

  /** The first string index whose byte offset is at least `byte`. */
  toIndex(byte: number): number {
    let low = 0;
    let high = this.text.length;
    while (low < high) {
      const mid = (low + high) >> 1;
      if (this.starts[mid]! < byte) low = mid + 1;
      else high = mid;
    }
    return low;
  }

  toByte(index: number): number {
    return this.starts[Math.min(Math.max(index, 0), this.text.length)]!;
  }

  /** A byte range as a string range, or null when there is none. */
  range(range: [number, number] | null | undefined): [number, number] | null {
    if (!range) return null;
    return [this.toIndex(range[0]), this.toIndex(range[1])];
  }
}

function isLow(unit: number): boolean {
  return unit >= 0xdc00 && unit <= 0xdfff;
}
