// Connect actual advancing players; decorative only. All content works without JS.
(() => {
  for (const board of document.querySelectorAll('.bracket-board')) {
    const svg = board.querySelector('svg');
    const draw = () => {
      svg.replaceChildren();
      if (matchMedia('(max-width:720px)').matches) return;
      const origin = board.getBoundingClientRect();
      svg.setAttribute('viewBox', `0 0 ${origin.width} ${origin.height}`);
      const columns = [...board.querySelectorAll('.bracket-column')];
      for (let i = 1; i < columns.length; i++) {
        const previous = [...columns[i - 1].querySelectorAll('[data-player]')];
        for (const target of columns[i].querySelectorAll('[data-player]')) {
          const source = previous.find(el => el.dataset.player === target.dataset.player);
          if (!source) continue;
          const a = source.getBoundingClientRect(), b = target.getBoundingClientRect();
          const x1 = a.right - origin.left, x2 = b.left - origin.left;
          const y1 = a.top + a.height / 2 - origin.top, y2 = b.top + b.height / 2 - origin.top;
          const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
          path.setAttribute('d', `M${x1},${y1} C${x1 + 32},${y1} ${x2 - 32},${y2} ${x2},${y2}`);
          if (target.dataset.wildcard === 'true') path.classList.add('wildcard');
          svg.append(path);
        }
      }
    };
    new ResizeObserver(draw).observe(board);
    document.fonts.ready.then(draw);
    draw();
  }
})();
