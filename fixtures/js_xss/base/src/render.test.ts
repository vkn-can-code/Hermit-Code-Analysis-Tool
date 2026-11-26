import { describe, it, expect } from 'vitest';
import { renderComment, escapeHtml } from './render';

describe('render', () => {
  it('renders author and body', () => {
    const el = { textContent: '', innerHTML: '' } as any;
    renderComment(el, 'ann', 'hi');
    expect(el.textContent).toContain('ann');
  });
  it('escapes html', () => {
    expect(escapeHtml('<b>')).toBe('&lt;b&gt;');
  });
});
