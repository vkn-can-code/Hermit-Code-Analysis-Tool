import { renderComment } from './render';

export function showComments(root: HTMLElement, items: { author: string; body: string }[]) {
  for (const c of items) {
    const li = document.createElement('li');
    renderComment(li, c.author, c.body);
    root.appendChild(li);
  }
}
