// Decoder (and, for the browser stub, encoder) of the engine's viewport
// binary form, little-endian:
//   u64 first_row, u32 first_col, u32 rows, u32 cols,
//   then per cell: u32 (byte_len << 1 | truncated), UTF-8 bytes;
//   with a filter on, then u32 rows and that many u64 document row numbers.

export interface ViewportData {
  firstRow: number;
  firstCol: number;
  rows: number;
  cols: number;
  cells: string[];
  truncated: Uint8Array;
  /** With a filter on: each row's number in the whole document (0-based). */
  rowNumbers?: number[];
}

const decoder = new TextDecoder("utf-8");
const encoder = new TextEncoder();

/**
 * Read a viewport reply. Tauri normally hands over the raw bytes, but when
 * its fast channel is unavailable (blocked by the page's security policy, an
 * old WebView) the fallback channel sends them as a plain list of numbers.
 */
export function decodeViewport(reply: ArrayBuffer | ArrayLike<number>): ViewportData {
  const buf = reply instanceof ArrayBuffer ? reply : Uint8Array.from(reply).buffer;
  const view = new DataView(buf);
  if (buf.byteLength < 20) throw new Error("viewport reply too short");
  const firstRow = Number(view.getBigUint64(0, true));
  const firstCol = view.getUint32(8, true);
  const rows = view.getUint32(12, true);
  const cols = view.getUint32(16, true);
  const n = rows * cols;
  // Every cell takes at least 4 bytes, so a damaged header cannot make us
  // reserve more than the reply could hold.
  if (n * 4 > buf.byteLength - 20) throw new Error("viewport reply cut short");
  const bytes = new Uint8Array(buf);
  const cells: string[] = new Array<string>(n);
  const truncated = new Uint8Array(n);
  let p = 20;
  for (let i = 0; i < n; i++) {
    if (p + 4 > buf.byteLength) throw new Error("viewport reply cut short");
    const head = view.getUint32(p, true);
    p += 4;
    const len = head >>> 1;
    if (p + len > buf.byteLength) throw new Error("viewport reply cut short");
    cells[i] = len === 0 ? "" : decoder.decode(bytes.subarray(p, p + len));
    truncated[i] = head & 1;
    p += len;
  }
  let rowNumbers: number[] | undefined;
  if (p + 4 <= buf.byteLength) {
    const k = view.getUint32(p, true);
    p += 4;
    if (p + k * 8 > buf.byteLength) throw new Error("viewport reply cut short");
    rowNumbers = [];
    for (let i = 0; i < k; i++) rowNumbers.push(Number(view.getBigUint64(p + i * 8, true)));
  }
  return { firstRow, firstCol, rows, cols, cells, truncated, rowNumbers };
}

export function encodeViewport(v: ViewportData): ArrayBuffer {
  const parts = v.cells.map((c) => encoder.encode(c));
  const nums = v.rowNumbers;
  const size = 20 + parts.reduce((s, p) => s + 4 + p.length, 0) + (nums ? 4 + nums.length * 8 : 0);
  const buf = new ArrayBuffer(size);
  const view = new DataView(buf);
  const out = new Uint8Array(buf);
  view.setBigUint64(0, BigInt(v.firstRow), true);
  view.setUint32(8, v.firstCol, true);
  view.setUint32(12, v.rows, true);
  view.setUint32(16, v.cols, true);
  let p = 20;
  parts.forEach((part, i) => {
    view.setUint32(p, ((part.length << 1) | (v.truncated[i] ? 1 : 0)) >>> 0, true);
    p += 4;
    out.set(part, p);
    p += part.length;
  });
  if (nums) {
    view.setUint32(p, nums.length, true);
    nums.forEach((n, i) => view.setBigUint64(p + 4 + i * 8, BigInt(n), true));
  }
  return buf;
}
