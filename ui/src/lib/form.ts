// `form` requests: the HTML `<form>` of omp's `ask` tool (or an extension's `ui.formDialog`),
// rendered from an allowlist and answered with its data (`{ data }` by control name).

export type FormValue = string | number | string[];

const HTML_TAGS = new Set([
  'form', 'p', 'br', 'hr', 'div', 'span', 'small', 'strong', 'b', 'em', 'i', 'u', 's', 'code', 'pre', 'kbd', 'blockquote',
  'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'ul', 'ol', 'li', 'dl', 'dt', 'dd', 'table', 'caption', 'thead', 'tbody', 'tfoot', 'tr', 'th', 'td',
  'fieldset', 'legend', 'label', 'input', 'select', 'option', 'optgroup', 'textarea', 'img', 'details', 'summary',
]);
const SVG_TAGS = new Set([
  'svg', 'g', 'path', 'rect', 'circle', 'ellipse', 'line', 'polyline', 'polygon', 'text', 'tspan', 'title', 'desc', 'defs', 'marker',
  'lineargradient', 'radialgradient', 'stop',
]);
const ATTRS = new Set([
  'name', 'value', 'type', 'checked', 'selected', 'multiple', 'placeholder', 'min', 'max', 'step', 'pattern', 'required', 'minlength',
  'maxlength', 'rows', 'cols', 'alt', 'width', 'height', 'colspan', 'rowspan', 'open', 'label', 'title', 'start', 'reversed',
]);
const SVG_ATTRS = new Set([
  'viewbox', 'width', 'height', 'x', 'y', 'x1', 'y1', 'x2', 'y2', 'cx', 'cy', 'r', 'rx', 'ry', 'd', 'points', 'fill', 'fill-opacity',
  'stroke', 'stroke-width', 'stroke-dasharray', 'stroke-linecap', 'stroke-linejoin', 'stroke-opacity', 'opacity', 'transform',
  'text-anchor', 'dominant-baseline', 'font-size', 'font-weight', 'font-family', 'dx', 'dy', 'id', 'marker-start', 'marker-mid',
  'marker-end', 'markerwidth', 'markerheight', 'refx', 'refy', 'orient', 'markerunits', 'offset', 'stop-color', 'stop-opacity',
  'gradientunits', 'xmlns', 'preserveaspectratio',
]);
const INPUT_TYPES = new Set(['text', 'radio', 'checkbox', 'number', 'date', 'email', 'url', 'time', 'datetime-local', 'month', 'week', 'range', 'tel', 'search', 'color']);
/** SVG values may reference only local ids (`url(#arrow)`), never another document. */
const SAFE_SVG_VALUE = /^(?!.*url\(\s*(?!['"]?#))[^<>]*$/i;
const SAFE_IMG = /^(https?:|data:image\/(png|jpe?g|gif|webp);)/i;

/** The form's first `<form>` (or its whole body wrapped in one), with everything outside the allowlist removed. */
export function sanitizeForm(html: string): HTMLFormElement {
  const doc = new DOMParser().parseFromString(html, 'text/html');
  const source = doc.querySelector('form') ?? doc.body;
  const form = document.createElement('form');
  for (const child of Array.from(source.childNodes)) {
    const clean = sanitizeNode(child, false);
    if (clean) form.append(clean);
  }
  addOthers(form);
  return form;
}

function sanitizeNode(node: Node, inSvg: boolean): Node | null {
  if (node.nodeType === Node.TEXT_NODE) return document.createTextNode(node.textContent ?? '');
  if (node.nodeType !== Node.ELEMENT_NODE) return null;
  const el = node as Element;
  const tag = el.localName.toLowerCase();
  const svg = inSvg || tag === 'svg';
  if (svg ? !SVG_TAGS.has(tag) : !HTML_TAGS.has(tag) || tag === 'form') {
    // An unknown wrapper keeps its text and allowed children; script-like content goes entirely.
    if (['script', 'style', 'template', 'iframe', 'object', 'embed', 'foreignobject', 'button', 'noscript'].includes(tag)) return null;
    const frag = document.createDocumentFragment();
    for (const child of Array.from(el.childNodes)) {
      const clean = sanitizeNode(child, inSvg);
      if (clean) frag.append(clean);
    }
    return frag;
  }
  if (tag === 'input') {
    const type = (el.getAttribute('type') ?? 'text').toLowerCase();
    if (!INPUT_TYPES.has(type)) return null;
  }
  const out = svg ? document.createElementNS('http://www.w3.org/2000/svg', el.localName) : document.createElement(tag);
  for (const attr of Array.from(el.attributes)) {
    const name = attr.name.toLowerCase();
    if (name.startsWith('on')) continue;
    if (svg) {
      if (SVG_ATTRS.has(name) && SAFE_SVG_VALUE.test(attr.value)) out.setAttribute(attr.name, attr.value);
    } else if (ATTRS.has(name)) out.setAttribute(name, attr.value);
    else if (name === 'src' && tag === 'img' && SAFE_IMG.test(attr.value.trim())) out.setAttribute('src', attr.value.trim());
  }
  if (tag === 'img') {
    out.setAttribute('referrerpolicy', 'no-referrer');
    out.setAttribute('loading', 'lazy');
  }
  for (const child of Array.from(el.childNodes)) {
    const clean = sanitizeNode(child, svg);
    if (clean) out.append(clean);
  }
  return out;
}

/** Controls named `name` of one kind. */
function group(form: HTMLFormElement, type: 'radio' | 'checkbox', name: string): HTMLInputElement[] {
  return Array.from(form.querySelectorAll<HTMLInputElement>(`input[type="${type}"]`)).filter((i) => i.name === name);
}

/** Radio groups and checkbox groups (shared name) get an "Other" answer after their last option, as omp promises the agent. */
function addOthers(form: HTMLFormElement) {
  const seen = new Set<string>();
  for (const input of Array.from(form.querySelectorAll<HTMLInputElement>('input[type="radio"], input[type="checkbox"]'))) {
    const type = input.type as 'radio' | 'checkbox';
    const name = input.name;
    if (!name || seen.has(`${type}:${name}`)) continue;
    seen.add(`${type}:${name}`);
    const members = group(form, type, name);
    if (type === 'checkbox' && members.length < 2) continue;
    const last = members[members.length - 1];
    const anchor = last.closest('label') ?? last;
    const label = document.createElement('label');
    label.className = 'other';
    const choice = document.createElement('input');
    choice.type = type;
    choice.name = name;
    choice.value = '';
    choice.dataset.other = '';
    const text = document.createElement('input');
    text.type = 'text';
    text.placeholder = 'Other — type your own answer';
    text.dataset.otherText = '';
    // Typing picks it; clearing the text leaves the choice to the user.
    text.addEventListener('input', () => {
      if (text.value.trim()) choice.checked = true;
    });
    label.append(choice, text);
    anchor.after(label);
  }
}

/** The submitted data by control name, as omp's `form` answer wants it. */
export function formData(form: HTMLFormElement): Record<string, FormValue> {
  const data: Record<string, FormValue> = {};
  const otherText = (input: HTMLInputElement) =>
    (input.closest('label')?.querySelector<HTMLInputElement>('input[data-other-text]')?.value ?? '').trim();
  for (const el of Array.from(form.elements)) {
    if (el instanceof HTMLInputElement) {
      const name = el.name;
      if (!name || el.dataset.otherText !== undefined) continue;
      if (el.type === 'radio') {
        if (!el.checked) continue;
        const value = el.dataset.other !== undefined ? otherText(el) : el.value;
        if (value) data[name] = value;
      } else if (el.type === 'checkbox') {
        // A shared name is a group: `addOthers` gave it the "Other" choice.
        if (group(form, 'checkbox', name).some((m) => m.dataset.other !== undefined)) {
          const list = Array.isArray(data[name]) ? (data[name] as string[]) : (data[name] = []);
          if (!el.checked) continue;
          const value = el.dataset.other !== undefined ? otherText(el) : el.value;
          if (value) list.push(value);
        } else if (el.checked) data[name] = el.value;
      } else if (el.type === 'number' || el.type === 'range') {
        if (el.value !== '' && !Number.isNaN(el.valueAsNumber)) data[name] = el.valueAsNumber;
      } else data[name] = el.value;
    } else if (el instanceof HTMLSelectElement) {
      if (!el.name) continue;
      data[el.name] = el.multiple ? Array.from(el.selectedOptions).map((o) => o.value) : el.value;
    } else if (el instanceof HTMLTextAreaElement) {
      if (el.name) data[el.name] = el.value;
    }
  }
  return data;
}

/** One line for lists: the form's first legend or paragraph. */
export function formSummary(html: string): string {
  const doc = new DOMParser().parseFromString(html, 'text/html');
  const first = doc.querySelector('legend, p, h1, h2, h3, h4, label');
  const text = (first?.textContent ?? doc.body.textContent ?? '').replace(/\s+/g, ' ').trim();
  return text || 'Form';
}
