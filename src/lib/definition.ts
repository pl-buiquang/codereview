/** Identifier (`foo`, `$el`, `empty?`) containing `offset` in `text`, or null when it falls on punctuation/space. */
export function identifierAt(text: string, offset: number): string | null {
  const isWord = (c: string | undefined) => c !== undefined && /[A-Za-z0-9_$]/.test(c);
  let start = offset;
  // A caret at the right edge of a word still belongs to that word.
  if (!isWord(text[start]) && isWord(text[start - 1])) start -= 1;
  if (!isWord(text[start])) return null;
  let end = start;
  while (isWord(text[start - 1])) start -= 1;
  while (isWord(text[end + 1])) end += 1;
  let word = text.slice(start, end + 1);
  if ((text[end + 1] === "?" || text[end + 1] === "!") && /^[A-Za-z_]/.test(word)) word += text[end + 1];
  return /^[A-Za-z_$]/.test(word) ? word : null;
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

/** The identifier under a mouse event's pointer. */
export function identifierUnderPointer(e: { clientX: number; clientY: number }): string | null {
  const caret = caretAt(e.clientX, e.clientY);
  if (!caret || caret.node.nodeType !== Node.TEXT_NODE) return null;
  return identifierAt(caret.node.textContent ?? "", caret.offset);
}
