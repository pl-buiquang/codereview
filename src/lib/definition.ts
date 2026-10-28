const isWord = (c: string | undefined) => c !== undefined && /[A-Za-z0-9_$]/.test(c);

/**
 * Identifier (`foo`, `$el`, `empty?`) containing `offset` in `text`, with its `[start, end)`
 * range, or null when it falls on punctuation/space/a number.
 */
export function identifierAt(
  text: string,
  offset: number,
): { word: string; start: number; end: number } | null {
  let start = offset;
  // A caret at the right edge of a word still belongs to that word.
  if (!isWord(text[start]) && isWord(text[start - 1])) start -= 1;
  if (!isWord(text[start])) return null;
  let last = start;
  while (isWord(text[start - 1])) start -= 1;
  while (isWord(text[last + 1])) last += 1;
  let end = last + 1;
  let word = text.slice(start, end);
  if ((text[end] === "?" || text[end] === "!") && /^[A-Za-z_]/.test(word)) {
    word += text[end];
    end += 1;
  }
  return /^[A-Za-z_$]/.test(word) ? { word, start, end } : null;
}

/** Text node + offset under the mouse, via the WebKit/Blink or standard caret APIs. */
function caretAt(x: number, y: number): { node: Node; offset: number } | null {
  const doc = document as Document & {
    caretPositionFromPoint?: (x: number, y: number) => { offsetNode: Node; offset: number } | null;
  };
  if (doc.caretPositionFromPoint) {
    const p = doc.caretPositionFromPoint(x, y);
    return p ? { node: p.offsetNode, offset: p.offset } : null;
  }
  const r = document.caretRangeFromPoint?.(x, y);
  return r ? { node: r.startContainer, offset: r.startOffset } : null;
}

export interface IdentifierUnderPointer {
  word: string;
  node: Text;
  start: number;
  end: number;
}

/**
 * The identifier under a mouse event's pointer, and where it sits in the DOM. The caret APIs
 * snap to the nearest text, so the pointer must really be over the word's glyphs (checked against
 * the word's client rects) — otherwise the empty rest of a line would count.
 */
export function identifierUnderPointer(e: {
  clientX: number;
  clientY: number;
}): IdentifierUnderPointer | null {
  const caret = caretAt(e.clientX, e.clientY);
  if (!caret || caret.node.nodeType !== Node.TEXT_NODE) return null;
  const node = caret.node as Text;
  const hit = identifierAt(node.textContent ?? "", caret.offset);
  if (!hit) return null;
  const range = document.createRange();
  range.setStart(node, hit.start);
  range.setEnd(node, hit.end);
  const over = Array.from(range.getClientRects()).some(
    (r) => e.clientX >= r.left && e.clientX <= r.right && e.clientY >= r.top && e.clientY <= r.bottom,
  );
  return over ? { word: hit.word, node, start: hit.start, end: hit.end } : null;
}

/** Name of the CSS custom highlight used for the Cmd+hover link (`::highlight(...)`). */
export const DEFINITION_HIGHLIGHT = "definition-link";

/** Underline `target` as a link. No-op where the CSS Custom Highlight API is missing. */
export function showDefinitionLink(target: IdentifierUnderPointer | null) {
  if (typeof Highlight === "undefined" || !("highlights" in CSS)) return;
  if (!target) {
    CSS.highlights.delete(DEFINITION_HIGHLIGHT);
    return;
  }
  const range = document.createRange();
  range.setStart(target.node, target.start);
  range.setEnd(target.node, target.end);
  CSS.highlights.set(DEFINITION_HIGHLIGHT, new Highlight(range));
}
