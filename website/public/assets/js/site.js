// coocoo.dev — demos and small enhancements. No dependencies.
(() => {
  'use strict';
  const d = document;
  const $ = (s, r = d) => r.querySelector(s);
  const $$ = (s, r = d) => Array.from(r.querySelectorAll(s));
  const reduce = matchMedia('(prefers-reduced-motion: reduce)').matches;
  d.documentElement.classList.add('js');

  /* ---------- demo scheduler ----------
     Every demo gets timers that only count down while its element is on screen and the tab is
     visible; CSS animations inside it pause through the `.paused` class. */
  const demos = [];
  class Demo {
    constructor(el) {
      this.el = el;
      this.running = false;
      this.visible = false;
      this.timers = new Set();
      this.toggles = [];
      el.classList.add('paused');
    }
    wait(ms) {
      return new Promise((resolve) => {
        const t = { left: ms, resolve, id: 0, at: 0 };
        this.timers.add(t);
        if (this.running) this.arm(t);
      });
    }
    every(ms, fn) {
      (async () => { for (;;) { await this.wait(ms); fn(); } })();
    }
    onToggle(fn) { this.toggles.push(fn); }
    arm(t) {
      t.at = performance.now();
      t.id = setTimeout(() => { this.timers.delete(t); t.resolve(); }, t.left);
    }
    set(on) {
      if (on === this.running) return;
      this.running = on;
      this.el.classList.toggle('paused', !on);
      for (const t of this.timers) {
        if (on) this.arm(t);
        else { clearTimeout(t.id); t.left = Math.max(0, t.left - (performance.now() - t.at)); }
      }
      this.toggles.forEach((fn) => fn(on));
    }
    sync() { this.set(this.visible && !d.hidden); }
  }
  const io = new IntersectionObserver((entries) => {
    for (const e of entries) {
      const demo = demos.find((x) => x.el === e.target);
      if (demo) { demo.visible = e.isIntersecting; demo.sync(); }
    }
  }, { threshold: 0.15 });
  function demo(el) {
    const x = new Demo(el);
    demos.push(x);
    io.observe(el);
    return x;
  }
  d.addEventListener('visibilitychange', () => demos.forEach((x) => x.sync()));

  /* ---------- reveal on scroll, cursor glow ---------- */
  const revealIO = new IntersectionObserver((entries) => {
    for (const e of entries) if (e.isIntersecting) { e.target.classList.add('vis'); revealIO.unobserve(e.target); }
  }, { threshold: 0.12 });
  $$('.reveal').forEach((el) => revealIO.observe(el));

  $$('.glow').forEach((card) => card.addEventListener('pointermove', (e) => {
    const r = card.getBoundingClientRect();
    card.style.setProperty('--mx', `${e.clientX - r.left}px`);
    card.style.setProperty('--my', `${e.clientY - r.top}px`);
  }));

  /* ---------- download buttons: iPadOS reports a Mac user agent ---------- */
  if (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1) {
    $$('[data-download]').forEach((a) => {
      a.href = '#start';
      $('[data-download-label]', a).textContent = 'Get it on your computer';
    });
    $$('[data-mobile-note]').forEach((p) => { p.hidden = false; });
  }

  /* ---------- hero ---------- */
  const hero = $('.hero');
  const stage = $('[data-demo="hero"]');
  if (hero && stage) heroDemo(demo(hero), stage);

  function heroDemo(D, stage) {
    requestAnimationFrame(() => setTimeout(() => stage.classList.add('in'), 120));
    const q = (s) => $(s, stage);
    const S = { w: 3, n: 0, r: 1 };
    const badge = q('[data-badge]');
    const askEl = q('[data-ask]');
    const tryEl = q('[data-try]');
    const toastEl = q('[data-toast]');
    const row0 = q('[data-row="0"]');
    const announce = q('[data-announce]');

    const counts = () => {
      q('[data-count-working]').textContent = S.w;
      q('[data-count-need]').textContent = S.n;
      q('[data-count-ready]').textContent = S.r;
      badge.textContent = S.n;
      badge.classList.toggle('on', S.n > 0);
      q('[data-queue-note]').textContent = S.n ? `· ${S.n} waiting` : '· nothing right now';
    };
    const prog = (i, step) => {
      const bar = q(`[data-prog="${i}"]`);
      if (!bar) return;
      bar.className = `s${step}`;
      q(`[data-prog-t="${i}"]`).textContent = `${step}/5`;
    };
    const dot = (i, cls) => {
      q(`[data-dot="${i}"]`).className = `dot ${cls}`;
      const side = q(`[data-sdot="${i}"]`);
      if (side) side.className = `dot ${cls}`;
    };
    const sub = (i, html) => { q(`[data-sub="${i}"]`).innerHTML = html; };
    const working = (i, step, text) =>
      `<span class="todo"><span class="prog"><i class="s${step}" data-prog="${i}"></i></span><span data-prog-t="${i}">${step}/5</span></span><span>${text}</span>`;
    const done = (files, plus, minus, text) =>
      `<span class="diff">${files} files <span class="p">+${plus}</span> <span class="m">−${minus}</span></span><span>${text}</span>`;
    const toast = (text) => {
      q('[data-toast-text]').textContent = text;
      toastEl.classList.add('show');
      D.wait(2800).then(() => toastEl.classList.remove('show'));
    };

    // live host bars
    const hostBars = $$('[data-host]', stage).map((el) => {
      const [kind, base, total, unit] = el.dataset.host.split(':');
      return { el, value: el.parentElement.nextElementSibling, kind, base: +base, total: +total, unit };
    });
    const hosts = () => {
      for (const h of hostBars) {
        const v = Math.max(4, Math.min(96, h.base + (Math.random() * 24 - 12)));
        h.el.style.setProperty('--v', (v / 100).toFixed(3));
        h.value.textContent = h.kind === 'cpu' ? `${Math.round(v)}%` : `${(v * h.total / 100).toFixed(1)}${h.unit}`;
      }
    };
    hosts();

    // rotating word in the headline
    const rot = $('[data-rotate]', hero);
    if (rot && !reduce) {
      const words = rot.dataset.rotate.split('|');
      let wi = 0;
      D.every(2600, () => {
        wi = (wi + 1) % words.length;
        rot.classList.add('out');
        setTimeout(() => {
          rot.textContent = words[wi];
          rot.classList.remove('out');
          rot.classList.add('in');
          void rot.offsetWidth;
          rot.classList.remove('in');
        }, 230);
      });
    }
    if (!reduce) D.every(1800, hosts);

    // answering: click, or 1/2/3 while the demo is on screen
    let answerFn = null;
    const ask = () => new Promise((resolve) => { answerFn = (v) => { answerFn = null; resolve(v); }; });
    $$('[data-answer]', stage).forEach((b) => b.addEventListener('click', () => answerFn && answerFn(b.dataset.answer)));
    d.addEventListener('keydown', (e) => {
      if (!answerFn || e.altKey || e.ctrlKey || e.metaKey || e.repeat) return;
      const choice = { 1: 'yes', 2: 'dry', 3: 'no' }[e.key];
      if (!choice) return;
      const t = d.activeElement && d.activeElement.tagName;
      if (t === 'INPUT' || t === 'TEXTAREA' || t === 'SELECT') return;
      const r = stage.getBoundingClientRect();
      if (r.bottom < 80 || r.top > innerHeight - 80) return;
      e.preventDefault();
      answerFn(choice);
    });

    const placeTry = () => {
      const last = q('[data-answer="no"]').getBoundingClientRect();
      const s = stage.getBoundingClientRect();
      tryEl.style.transform = `translate(${Math.round(last.right - s.left + 16)}px, ${Math.round(last.top - s.top + last.height / 2 - 15)}px)`;
    };
    const AFTER = {
      yes: ['Migrating staging…', [6, 212, 48, 'Checkout sessions on v3, webhooks verified. Ready for review.']],
      dry: ['Dry run: 2 tables, 0 conflicts. Migrating…', [6, 212, 48, 'Dry run clean, then migrated. Ready for review.']],
      no: ['Skipping the migration, writing it up instead', [5, 188, 41, 'Migration documented in MIGRATION.md, not run. Ready for review.']],
    };

    const reset = () => {
      S.w = 3; S.n = 0; S.r = 1; counts();
      dot(0, 'b work'); dot(1, 'b work');
      sub(0, working(0, 2, 'Editing <code>billing/checkout.ts</code>'));
      sub(1, working(1, 3, 'Checking contrast in the browser'));
      prog(2, 1);
      row0.classList.remove('sel');
    };
    const openAsk = (first) => {
      S.w = 2; S.n = 1; counts();
      dot(0, 'o pulse');
      row0.classList.add('sel');
      sub(0, '<span class="wait">Waiting for you</span>');
      askEl.hidden = false;
      placeTry();
      tryEl.classList.add('show');
      if (first) announce.textContent = 'Demo: the agent “Migrate billing to Stripe Checkout v3” asks whether to run the database migration against staging. Answer with the buttons.';
    };
    const closeAsk = () => {
      const hadFocus = askEl.contains(d.activeElement);
      askEl.hidden = true;
      tryEl.classList.remove('show');
      if (hadFocus) { row0.tabIndex = -1; row0.focus({ preventScroll: true }); }
    };
    const finishOther = () => {
      dot(1, 'g'); S.w--; S.r++; counts();
      sub(1, done(3, 10, 5, 'Ready for review · passes WCAG AA in both themes'));
    };
    const finishMain = (a) => {
      dot(0, 'g'); S.w--; S.r++; counts();
      row0.classList.remove('sel');
      sub(0, done(...AFTER[a === 'auto' ? 'yes' : a][1]));
    };

    if (reduce) {
      // Static end states; the question stays answerable.
      reset();
      prog(1, 5); finishOther();
      openAsk(true);
      ask().then((a) => { closeAsk(); S.n = 0; S.w++; counts(); toast('Answered from the overview. The agent continues.'); finishMain(a); });
      return;
    }

    (async () => {
      for (let first = true; ; first = false) {
        reset();
        await D.wait(1400); prog(0, 3); prog(2, 2);
        await D.wait(1200);
        openAsk(first);
        const other = (async () => {
          await D.wait(1600); prog(1, 4);
          await D.wait(1600); prog(1, 5);
          await D.wait(600); finishOther();
        })();
        const a = await Promise.race([ask(), D.wait(9000).then(() => 'auto')]);
        answerFn = null;
        closeAsk();
        toast(a === 'auto' ? 'Answered from your phone. The agent continues.' : 'Answered from the overview. The agent continues.');
        S.n = 0; S.w++; counts();
        dot(0, 'b work');
        sub(0, working(0, 3, AFTER[a === 'auto' ? 'yes' : a][0]));
        await D.wait(1800); prog(0, 4);
        await D.wait(1500); prog(0, 5);
        await D.wait(500); await other;
        finishMain(a);
        await D.wait(7000);
      }
    })();
  }

  /* ---------- proof numbers count up ---------- */
  if (!reduce) {
    const countIO = new IntersectionObserver((entries) => {
      for (const e of entries) {
        if (!e.isIntersecting) continue;
        countIO.unobserve(e.target);
        const el = e.target;
        const to = parseFloat(el.dataset.countTo);
        const dec = (el.dataset.countTo.split('.')[1] || '').length;
        const t0 = performance.now();
        const step = (t) => {
          const p = Math.min(1, (t - t0) / 1400);
          el.textContent = (to * (1 - (1 - p) ** 3)).toFixed(dec);
          if (p < 1) requestAnimationFrame(step);
        };
        el.textContent = (0).toFixed(dec);
        requestAnimationFrame(step);
      }
    }, { threshold: 0.6 });
    $$('[data-count-to]').forEach((el) => countIO.observe(el));
  }

  /* ---------- 01 triage: j/k moving through the queue ---------- */
  const tri = $('[data-demo="triage"]');
  if (tri && !reduce) {
    const D = demo(tri);
    const rows = $$('[data-tri]', tri);
    let i = 0;
    let dir = 1;
    D.every(2000, () => {
      if (i + dir < 0 || i + dir >= rows.length) dir = -dir;
      i += dir;
      rows.forEach((r, n) => r.classList.toggle('sel', n === i));
      const key = $(`kbd[data-key="${dir > 0 ? 'j' : 'k'}"]`, tri);
      key.classList.add('press');
      D.wait(260).then(() => key.classList.remove('press'));
    });
  }

  /* ---------- 02 fleet graph ---------- */
  const fleet = $('[data-demo="fleet"]');
  if (fleet) {
    const D = demo(fleet);
    const svg = $('[data-lines]', fleet);
    const NS = 'http://www.w3.org/2000/svg';
    const draw = () => {
      const box = fleet.getBoundingClientRect();
      const pts = $$('.node', fleet).map((n) => {
        const b = n.getBoundingClientRect();
        return [b.left - box.left + b.width / 2, b.top - box.top + b.height / 2];
      });
      svg.setAttribute('viewBox', `0 0 ${box.width} ${box.height}`);
      svg.textContent = '';
      [[0, 1], [0, 2], [1, 2]].forEach(([a, b], i) => {
        const p = d.createElementNS(NS, 'path');
        p.setAttribute('id', `fleet-l${i}`);
        p.setAttribute('class', i === 2 ? 'ln dash' : 'ln');
        p.setAttribute('d', `M${pts[a][0]},${pts[a][1]} L${pts[b][0]},${pts[b][1]}`);
        svg.appendChild(p);
      });
      if (reduce) return;
      [0, 1, 0, 1].forEach((line, i) => {
        const c = d.createElementNS(NS, 'circle');
        c.setAttribute('r', '3');
        c.setAttribute('class', 'packet');
        const m = d.createElementNS(NS, 'animateMotion');
        m.setAttribute('dur', `${2.4 + i * 0.7}s`);
        m.setAttribute('begin', `${i * 0.6}s`);
        m.setAttribute('repeatCount', 'indefinite');
        if (i % 2) { m.setAttribute('keyPoints', '1;0'); m.setAttribute('keyTimes', '0;1'); m.setAttribute('calcMode', 'linear'); }
        const mp = d.createElementNS(NS, 'mpath');
        mp.setAttribute('href', `#fleet-l${line}`);
        m.appendChild(mp);
        c.appendChild(m);
        svg.appendChild(c);
      });
      if (!D.running) svg.pauseAnimations();
    };
    new ResizeObserver(draw).observe(fleet); // first callback draws, after layout
    D.onToggle((on) => (on ? svg.unpauseAnimations() : svg.pauseAnimations()));
    if (!reduce) {
      const states = ['', 'b', 'b', 'o', 'g'];
      const squares = $$('.mini i', fleet);
      D.every(900, () => {
        for (const sq of squares) if (Math.random() < 0.25) sq.className = states[Math.floor(Math.random() * states.length)];
      });
    }
  }

  /* ---------- 03 design-mode picker ---------- */
  const picker = $('[data-demo="picker"]');
  if (picker && !reduce) {
    const D = demo(picker);
    const boxes = $$('.pk', picker);
    const track = $('.cur-track', picker);
    let i = 0;
    D.every(2600, () => {
      i = (i + 1) % boxes.length;
      boxes.forEach((b, n) => b.classList.toggle('on', n === i));
      track.className = `cur-track c${i}`;
    });
  }

  /* ---------- 04 RAM bars ---------- */
  const ram = $('[data-demo="ram"]');
  if (ram) {
    const ramIO = new IntersectionObserver((entries, obs) => {
      if (entries.some((e) => e.isIntersecting)) { $$('.tr', ram).forEach((t) => t.classList.add('go')); obs.disconnect(); }
    }, { threshold: 0.4 });
    ramIO.observe(ram);
  }

  /* ---------- 05 kanban ---------- */
  const board = $('[data-board]');
  if (board && !reduce) {
    const D = demo(board.closest('[data-demo]'));
    const initial = [
      ['Homepage', 'refined · 2 mocks', 0], ['Changelog page', 'inbox · from RSS', 0],
      ['Dark mode for diff view', 'todo', 1], ['SSH peer docs', 'todo', 1],
      ['Host stats history', 'agent · studio', 2], ['Fix reconnect jitter', 'committed a41f2c9', 3],
    ];
    const moves = [[2, 'agent · studio', 2], [0, 'todo · refined', 1], [4, 'committed 9c03e1b', 3], [0, 'agent · gpu-box', 2], [2, 'committed 77b1d0e', 3]];
    const dots = ['dot', 'dot', 'dot b work', 'dot g'];
    let cards = initial.map((c) => c.slice());
    let step = 0;
    const cols = $$('.col', board);
    const render = (moved) => {
      cols.forEach((col, ci) => {
        $$('.kc', col).forEach((k) => k.remove());
        const items = cards.filter((c) => c[2] === ci);
        $('h3 span', col).textContent = items.length;
        for (const c of items) {
          const el = d.createElement('div');
          el.className = c === moved ? 'kc enter' : 'kc';
          el.append(c[0]);
          const who = d.createElement('div');
          who.className = 'who';
          const dt = d.createElement('span');
          dt.className = dots[ci];
          who.append(dt, c[1]);
          el.append(who);
          col.append(el);
        }
      });
    };
    D.every(2200, () => {
      if (step === moves.length) { cards = initial.map((c) => c.slice()); step = 0; render(null); return; }
      const [i, who, col] = moves[step++];
      const card = cards[i];
      card[1] = who; card[2] = col;
      cards.splice(i, 1); cards.push(card);
      // keep indices stable for the next moves
      cards = initial.map((c) => cards.find((x) => x[0] === c[0]));
      render(card);
    });
  }

  /* ---------- get started tabs ---------- */
  const tabs = $$('[role="tab"]');
  const select = (tab, focus) => {
    tabs.forEach((t) => {
      const on = t === tab;
      t.setAttribute('aria-selected', String(on));
      t.tabIndex = on ? 0 : -1;
      const panel = d.getElementById(t.getAttribute('aria-controls'));
      panel.hidden = !on;
      if (on && !reduce) {
        const lines = $$(':scope > div', panel);
        panel.classList.add('typing');
        lines.forEach((l) => l.classList.remove('on'));
        lines.forEach((l, n) => setTimeout(() => l.classList.add('on'), 120 + n * 380));
      }
    });
    if (focus) tab.focus();
  };
  tabs.forEach((t, i) => {
    t.addEventListener('click', () => select(t, false));
    t.addEventListener('keydown', (e) => {
      const n = { ArrowRight: 1, ArrowLeft: -1 }[e.key];
      if (n) { e.preventDefault(); select(tabs[(i + n + tabs.length) % tabs.length], true); }
    });
  });
  $$('[data-open-tab]').forEach((a) => a.addEventListener('click', () => {
    const t = d.getElementById(a.dataset.openTab);
    if (t) select(t, false);
  }));
  $$('[data-copy]').forEach((btn) => btn.addEventListener('click', async () => {
    const text = $$('[data-cmd]', btn.parentElement).map((c) => c.textContent).join('\n');
    try {
      await navigator.clipboard.writeText(text);
      btn.textContent = 'Copied';
    } catch {
      btn.textContent = 'Press ⌘C';
    }
    setTimeout(() => { btn.textContent = 'Copy'; }, 1600);
  }));
})();
