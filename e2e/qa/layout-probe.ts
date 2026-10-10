export type Check = 'page-scroll' | 'clipped-text' | 'escaping';
export type Finding = { check: Check; where: string; detail: string };

/**
 * Runs in the page. Reports horizontal scroll of the document or of the first
 * `<main>`, text wider than its box (unless truncated with an ellipsis or a
 * `title`), and boxes that a `hidden`/`clip` ancestor cuts off partially. Only
 * the nearest clipping ancestor counts, and the walk stops at the first
 * `auto`/`scroll` ancestor, whose content scrolls into view. A box entirely
 * outside its clipping ancestor is an off-canvas element and is not reported.
 */
export function collectLayoutFindings(): Finding[] {
  const findings: Finding[] = [];
  const TOLERANCE = 1;

  const describe = (el: Element): string => {
    const parts: string[] = [];
    let e: Element | null = el;
    for (let i = 0; e && e !== document.body && i < 3; i += 1, e = e.parentElement) {
      let s = e.tagName.toLowerCase();
      if (e.id) s += `#${e.id}`;
      const classes = Array.from(e.classList).slice(0, 2);
      if (classes.length > 0) s += `.${classes.join('.')}`;
      parts.unshift(s);
    }
    return parts.join(' > ');
  };

  const doc = document.documentElement;
  if (doc.scrollWidth > window.innerWidth) {
    findings.push({
      check: 'page-scroll',
      where: 'html',
      detail: `scrollWidth ${doc.scrollWidth} > innerWidth ${window.innerWidth}`,
    });
  }
  // The QA shell scrolls its `main`, not the document, so a page that is too
  // wide shows up there.
  const main = document.querySelector('main');
  if (main && main.scrollWidth > main.clientWidth + TOLERANCE) {
    findings.push({
      check: 'page-scroll',
      where: 'main',
      detail: `scrollWidth ${main.scrollWidth} > clientWidth ${main.clientWidth}`,
    });
  }

  for (const el of Array.from(document.body.querySelectorAll('*'))) {
    if (!(el instanceof HTMLElement)) continue;
    // The harness's own breadcrumbs are not under test.
    if (el.closest('nav[aria-label="QA navigation"]')) continue;
    const rect = el.getBoundingClientRect();
    const cs = getComputedStyle(el);
    if (rect.width === 0 || rect.height === 0 || cs.visibility === 'hidden') continue;

    const hasText = Array.from(el.childNodes).some(
      (n) => n.nodeType === Node.TEXT_NODE && (n.textContent ?? '').trim() !== '',
    );
    if (
      hasText &&
      el.clientWidth > TOLERANCE &&
      el.scrollWidth > el.clientWidth + TOLERANCE &&
      cs.textOverflow !== 'ellipsis' &&
      !el.hasAttribute('title')
    ) {
      findings.push({
        check: 'clipped-text',
        where: describe(el),
        detail: `scrollWidth ${el.scrollWidth} > clientWidth ${el.clientWidth}: "${(el.textContent ?? '').trim().slice(0, 40)}"`,
      });
    }

    if (cs.position === 'fixed') continue;
    let clip: HTMLElement | null = null;
    for (let a = el.parentElement; a && a !== document.body; a = a.parentElement) {
      const o = getComputedStyle(a);
      const axes = [o.overflowX, o.overflowY];
      if (axes.some((v) => v === 'auto' || v === 'scroll')) break;
      if (axes.some((v) => v === 'hidden' || v === 'clip')) {
        clip = a;
        break;
      }
    }
    if (!clip || clip.clientWidth <= TOLERANCE) continue;
    // A clip that draws an ellipsis truncates its content on purpose.
    if (getComputedStyle(clip).textOverflow === 'ellipsis') continue;

    const c = clip.getBoundingClientRect();
    const intersects = rect.right > c.left && rect.left < c.right && rect.bottom > c.top && rect.top < c.bottom;
    const exceeds =
      rect.left < c.left - TOLERANCE ||
      rect.right > c.right + TOLERANCE ||
      rect.top < c.top - TOLERANCE ||
      rect.bottom > c.bottom + TOLERANCE;
    if (intersects && exceeds) {
      findings.push({
        check: 'escaping',
        where: describe(el),
        detail: `box ${Math.round(rect.left)},${Math.round(rect.top)} ${Math.round(rect.width)}×${Math.round(rect.height)} cut off by ${describe(clip)}`,
      });
    }
  }

  return findings;
}
