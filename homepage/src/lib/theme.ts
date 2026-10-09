/*
  The theme switch: light, system or dark. "System" removes data-theme and follows
  prefers-color-scheme; the other two set it. The choice is kept in this browser only (Base.astro
  reads it before the first paint); a blocked store just means the choice lasts this visit.
*/
type Choice = 'light' | 'system' | 'dark';
const KEY = 'inkwell-theme';

export function themeSwitch(root: ParentNode = document) {
  const buttons = [...root.querySelectorAll<HTMLButtonElement>('[data-theme-set]')];
  if (!buttons.length) return;
  const html = document.documentElement;
  const mark = (c: Choice) => buttons.forEach((b) => b.setAttribute('aria-pressed', String(b.dataset.themeSet === c)));
  const current = (html.getAttribute('data-theme') as Choice | null) ?? 'system';
  mark(current);
  buttons.forEach((b) =>
    b.addEventListener('click', () => {
      const c = b.dataset.themeSet as Choice;
      if (c === 'system') html.removeAttribute('data-theme');
      else html.setAttribute('data-theme', c);
      try {
        if (c === 'system') localStorage.removeItem(KEY);
        else localStorage.setItem(KEY, c);
      } catch {
        /* storage blocked: the choice holds for this visit */
      }
      mark(c);
    }),
  );
}

/** Copy buttons: data-copy holds the text. */
export function copyButtons(root: ParentNode = document) {
  root.querySelectorAll<HTMLButtonElement>('[data-copy]').forEach((b) => {
    b.hidden = !navigator.clipboard;
    b.addEventListener('click', async () => {
      try {
        await navigator.clipboard.writeText(b.dataset.copy ?? '');
        b.textContent = 'Copied';
      } catch {
        b.textContent = 'Select and copy';
      }
      window.setTimeout(() => (b.textContent = 'Copy'), 2000);
    });
  });
}
