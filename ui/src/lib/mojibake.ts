// Fix mojibake, the screen's copy of `encoding::fix_mojibake` in the engine: UTF-8 text that was read as
// Windows-1252 and saved again (`cafÃ©` for `café`). Both copies run the same fixture file
// (crates/engine/src/testdata/mojibake.json), so they cannot drift.

/** What Windows-1252 puts at bytes 0x80 to 0x9F (undefined bytes are null). */
const CP1252_HIGH: (number | null)[] = [
  0x20ac, null, 0x201a, 0x0192, 0x201e, 0x2026, 0x2020, 0x2021, 0x02c6, 0x2030, 0x0160, 0x2039, 0x0152, null, 0x017d, null,
  null, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022, 0x2013, 0x2014, 0x02dc, 0x2122, 0x0161, 0x203a, 0x0153, null, 0x017e, 0x0178,
];
const BYTE_OF = new Map<number, number>();
CP1252_HIGH.forEach((cp, i) => {
  if (cp !== null) BYTE_OF.set(cp, 0x80 + i);
});

/** The byte a character stands for in a wrong Windows-1252 reading. The five bytes it leaves undefined (0x81, 0x8D,
 *  0x8F, 0x90, 0x9D) read as the Latin-1 control character, as most programs do. */
function byteOf(cp: number): number | null {
  if (cp < 0x80) return cp;
  if (cp <= 0x9f) return cp === 0x81 || cp === 0x8d || cp === 0x8f || cp === 0x90 || cp === 0x9d ? cp : null;
  if (cp <= 0xff) return cp;
  return BYTE_OF.get(cp) ?? null;
}

const isOdd = (cp: number): boolean =>
  (cp >= 0x80 && cp <= 0xff) ||
  cp === 0x152 || cp === 0x153 || cp === 0x160 || cp === 0x161 || cp === 0x178 || cp === 0x17d || cp === 0x17e || cp === 0x192 || cp === 0x2c6 || cp === 0x2dc ||
  (cp >= 0x2013 && cp <= 0x203a) || cp === 0x20ac || cp === 0x2122 || cp === 0xfffd;

const oddCount = (s: string): number => {
  let n = 0;
  for (const ch of s) if (isOdd(ch.codePointAt(0)!)) n++;
  return n;
};

/** The fixed text, or null when the text must be left alone (it is real text, or the fix is not sure). */
export function fixMojibake(text: string): string | null {
  // Plain ASCII: nothing to fix.
  if (!/[^\x00-\x7f]/.test(text)) return null;
  const bytes: number[] = [];
  for (const ch of text) {
    const b = byteOf(ch.codePointAt(0)!);
    if (b === null) return null;
    bytes.push(b);
  }
  let fixed: string;
  try {
    fixed = new TextDecoder("utf-8", { fatal: true }).decode(new Uint8Array(bytes));
  } catch {
    return null;
  }
  if (fixed === text) return null;
  for (const ch of fixed) {
    const cp = ch.codePointAt(0)!;
    // A control character (Cc) other than tab and the line breaks: not text.
    if ((cp < 0x20 || (cp >= 0x7f && cp <= 0x9f)) && cp !== 9 && cp !== 10 && cp !== 13) return null;
  }
  return oddCount(fixed) < oddCount(text) ? fixed : null;
}
