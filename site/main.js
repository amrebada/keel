// Keel landing page: tabs, copy buttons, scroll reveals. No dependencies.
(() => {
  // Tabs in the hero editor window
  document.querySelectorAll('[data-tabs]').forEach((win) => {
    const tabs = [...win.querySelectorAll('[role="tab"]')];
    const select = (tab) => {
      tabs.forEach((t) => {
        const on = t === tab;
        t.setAttribute('aria-selected', String(on));
        t.tabIndex = on ? 0 : -1;
        const panel = document.getElementById(t.getAttribute('aria-controls'));
        if (panel) panel.hidden = !on;
      });
    };
    tabs.forEach((tab, i) => {
      tab.addEventListener('click', () => { select(tab); stop(); });
      tab.addEventListener('keydown', (e) => {
        if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return;
        const next = tabs[(i + (e.key === 'ArrowRight' ? 1 : tabs.length - 1)) % tabs.length];
        select(next); next.focus(); stop();
      });
    });
    // Gently alternate until the visitor interacts
    const reduce = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    let timer = reduce ? null : setInterval(() => {
      const cur = tabs.findIndex((t) => t.getAttribute('aria-selected') === 'true');
      select(tabs[(cur + 1) % tabs.length]);
    }, 6000);
    function stop() { if (timer) { clearInterval(timer); timer = null; } }
    win.addEventListener('mouseenter', stop, { once: true });
  });

  // Copy to clipboard
  const flash = (el) => {
    el.classList.add('is-copied');
    setTimeout(() => el.classList.remove('is-copied'), 1400);
  };
  const copy = async (text, el) => {
    try { await navigator.clipboard.writeText(text); flash(el); } catch (_) { /* clipboard blocked */ }
  };
  document.querySelectorAll('[data-copy]').forEach((el) =>
    el.addEventListener('click', () => copy(el.dataset.copy, el)));
  document.querySelectorAll('[data-copy-target]').forEach((el) =>
    el.addEventListener('click', () => {
      const src = document.getElementById(el.dataset.copyTarget);
      if (!src) return;
      const cmds = src.innerText.split('\n')
        .filter((l) => l.startsWith('$') || l.startsWith('    '))
        .map((l) => l.replace(/^\$ /, ''))
        .join('\n');
      copy(cmds, el);
    }));

  // Reveal on scroll + animate bars
  const targets = document.querySelectorAll('.step, .card, .plugin, .bench__chart, .table-wrap, .chat, .split__body');
  targets.forEach((t) => t.classList.add('reveal'));
  if (!('IntersectionObserver' in window)) {
    targets.forEach((t) => t.classList.add('in'));
    document.querySelectorAll('[data-bars]').forEach((b) => b.classList.add('in'));
    return;
  }
  const io = new IntersectionObserver((entries) => {
    entries.forEach((e) => {
      if (!e.isIntersecting) return;
      e.target.classList.add('in');
      io.unobserve(e.target);
    });
  }, { threshold: 0.15, rootMargin: '0px 0px -40px 0px' });
  targets.forEach((t) => io.observe(t));
  document.querySelectorAll('[data-bars]').forEach((b) => io.observe(b));

  // Nav border once scrolled
  const nav = document.querySelector('.nav');
  const onScroll = () => nav.classList.toggle('is-scrolled', window.scrollY > 8);
  onScroll();
  window.addEventListener('scroll', onScroll, { passive: true });
})();
