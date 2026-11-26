export function escapeHtml(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

export function renderComment(el: HTMLElement, author: string, body: string): void {
  // Support **bold** markdown in comments
  const html = body.replace(/\*\*(.+?)\*\*/g, '<b>$1</b>');
  if (author) {
    el.innerHTML = `<strong>${escapeHtml(author)}</strong>: ${html}`;
  } else {
    el.innerHTML = html;
  }
}
