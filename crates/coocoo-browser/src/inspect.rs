//! Design mode: element highlight overlay and element inspection.

use serde_json::{Value, json};

/// Overlay colors: translucent blue content box, lighter padding, faint amber margin.
pub(crate) fn highlight_config() -> Value {
    json!({
        "showInfo": true,
        "showStyles": false,
        "showAccessibilityInfo": false,
        "contentColor": { "r": 59, "g": 130, "b": 246, "a": 0.22 },
        "paddingColor": { "r": 147, "g": 197, "b": 253, "a": 0.30 },
        "borderColor": { "r": 37, "g": 99, "b": 235, "a": 0.70 },
        "marginColor": { "r": 251, "g": 191, "b": 36, "a": 0.18 },
    })
}

/// Longest side of the element screenshot, in output pixels.
pub(crate) const SHOT_MAX_SIDE: f64 = 800.0;
pub(crate) const HTML_MAX: usize = 4096;
pub(crate) const TEXT_MAX: usize = 1024;

/// Called with `this` = node under the pointer. Returns `{selector, tag, html, text, styles,
/// box, clip}` where `box` is the viewport rect and `clip` the visible part in document
/// coordinates (for `Page.captureScreenshot`), or `null` when nothing is visible.
pub(crate) const INSPECT_FN: &str = r#"function () {
  const el = this.nodeType === 1 ? this : this.parentElement;
  if (!el) return null;
  const root = el.getRootNode();
  const scope = root.querySelectorAll ? root : document;
  const unique = (sel) => { try { return scope.querySelectorAll(sel).length === 1; } catch (_) { return false; } };
  const esc = CSS.escape;
  const stableId = (id) => id && id.length < 64 && !/\d{4,}|^:|:$|^[0-9]/.test(id);
  const stableClass = (c) => c.length < 40 && !c.includes(':') && !/^(svelte|css|sc|jsx|emotion|astro|ng)-/.test(c)
    && !/[0-9a-f]{6,}|\d{3,}|__[a-zA-Z0-9]{5,}/.test(c);
  const ATTRS = ['data-testid', 'data-test', 'data-qa', 'data-cy', 'name', 'aria-label', 'title', 'alt', 'placeholder', 'href'];
  const anchor = (e) => {
    const tag = e.localName;
    if (stableId(e.id)) { const s = '#' + esc(e.id); if (unique(s)) return s; }
    for (const a of ATTRS) {
      const v = e.getAttribute(a);
      if (v && v.length <= 80) { const s = `${tag}[${a}="${esc(v)}"]`; if (unique(s)) return s; }
    }
    return null;
  };
  const step = (e) => {
    const tag = e.localName;
    let s = tag;
    const cls = Array.from(e.classList).filter(stableClass).slice(0, 2);
    if (cls.length) s += cls.map((c) => '.' + esc(c)).join('');
    const p = e.parentElement;
    if (p) {
      const kids = Array.from(p.children);
      const ambiguous = kids.filter((x) => x !== e && x.matches(s)).length > 0;
      if (ambiguous) s += `:nth-of-type(${kids.filter((x) => x.localName === tag).indexOf(e) + 1})`;
    }
    return s;
  };
  const selector = (() => {
    const parts = [];
    for (let cur = el; cur && cur.nodeType === 1; cur = cur.parentElement) {
      const a = anchor(cur);
      if (a) { parts.unshift(a); return parts.join(' > '); }
      parts.unshift(step(cur));
      const sel = parts.join(' > ');
      if (unique(sel)) return sel;
    }
    return parts.join(' > ');
  })();

  const cs = getComputedStyle(el);
  const styles = {};
  const put = (p) => { const v = cs.getPropertyValue(p); if (v !== '') styles[p] = v; };
  ['display', 'position', 'width', 'height', 'margin', 'padding', 'font-family', 'font-size', 'font-weight',
   'line-height', 'color', 'background-color', 'border', 'border-radius', 'gap'].forEach(put);
  for (const p of ['opacity', 'box-shadow', 'z-index', 'overflow', 'text-align', 'background-image']) {
    const v = cs.getPropertyValue(p);
    if (v && v !== 'none' && v !== 'auto' && v !== '1' && v !== 'visible' && v !== 'start') styles[p] = v;
  }
  const disp = cs.display;
  if (disp.includes('flex')) ['flex-direction', 'flex-wrap', 'justify-content', 'align-items', 'align-content'].forEach(put);
  if (disp.includes('grid')) ['grid-template-columns', 'grid-template-rows', 'grid-auto-flow', 'justify-items', 'align-items'].forEach(put);
  const parent = el.parentElement && getComputedStyle(el.parentElement).display;
  if (parent && parent.includes('flex')) ['flex', 'align-self', 'order'].forEach(put);
  if (parent && parent.includes('grid')) ['grid-column', 'grid-row', 'justify-self', 'align-self'].forEach(put);

  const r = el.getBoundingClientRect();
  const vw = document.documentElement.clientWidth || innerWidth, vh = document.documentElement.clientHeight || innerHeight;
  const x0 = Math.max(0, r.left), y0 = Math.max(0, r.top);
  const x1 = Math.min(vw, r.right), y1 = Math.min(vh, r.bottom);
  const clip = x1 - x0 >= 1 && y1 - y0 >= 1
    ? { x: x0 + scrollX, y: y0 + scrollY, width: x1 - x0, height: y1 - y0 } : null;
  return {
    selector,
    tag: el.localName,
    html: el.outerHTML.slice(0, __HTML_MAX__),
    text: (el.innerText || el.textContent || '').slice(0, __TEXT_MAX__),
    styles,
    box: { x: r.x, y: r.y, width: r.width, height: r.height },
    clip,
  };
}"#;

/// `INSPECT_FN` with its limits substituted.
pub(crate) fn inspect_fn() -> String {
    INSPECT_FN.replace("__HTML_MAX__", &HTML_MAX.to_string()).replace("__TEXT_MAX__", &TEXT_MAX.to_string())
}

/// Screenshot scale so the longest side of `clip` is at most [`SHOT_MAX_SIDE`] pixels.
pub(crate) fn shot_scale(width: f64, height: f64) -> f64 {
    let side = width.max(height);
    if side <= SHOT_MAX_SIDE { 1.0 } else { SHOT_MAX_SIDE / side }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_caps_longest_side() {
        assert_eq!(shot_scale(100.0, 50.0), 1.0);
        assert_eq!(shot_scale(1600.0, 400.0), 0.5);
        assert!((shot_scale(300.0, 3200.0) * 3200.0 - 800.0).abs() < 1e-9);
    }

    #[test]
    fn inspect_fn_has_limits() {
        let f = inspect_fn();
        assert!(f.contains("slice(0, 4096)") && f.contains("slice(0, 1024)"));
        assert!(!f.contains("__HTML_MAX__") && !f.contains("__TEXT_MAX__"));
    }
}
