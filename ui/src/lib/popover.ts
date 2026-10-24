/** Gap kept between a popover and the edge of the window. */
const MARGIN = 8;

/**
 * Svelte action for an absolutely positioned popover: shifts it sideways so it stays inside the
 * window. A menu anchored to the right edge of a button in the middle of a phone's header would
 * otherwise stick out on the left.
 */
export function keepInView(node: HTMLElement) {
  const fit = () => {
    node.style.translate = '';
    const r = node.getBoundingClientRect();
    const width = document.documentElement.clientWidth;
    const dx = r.left < MARGIN ? MARGIN - r.left : r.right > width - MARGIN ? Math.max(MARGIN - r.left, width - MARGIN - r.right) : 0;
    if (dx) node.style.translate = `${dx}px 0`;
  };
  fit();
  window.addEventListener('resize', fit);
  return { destroy: () => window.removeEventListener('resize', fit) };
}
