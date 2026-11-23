// Opened straight from disk, a link to a folder can't show its index.html, so links point at the file itself.
if (location.protocol === 'file:') {
  for (const link of document.querySelectorAll('a[href]')) {
    const href = link.getAttribute('href');
    if (!/^[a-z]+:/i.test(href) && href.endsWith('/')) link.setAttribute('href', `${href}index.html`);
  }
}

// The widget on the page changes as you read, showing what each part describes.
const widget = document.querySelector('.widget');
const pinned = () => widget && getComputedStyle(widget).position === 'sticky';

if (widget && 'IntersectionObserver' in window) {
  const watch = new IntersectionObserver(
    (entries) => {
      if (!pinned()) return;
      for (const entry of entries) {
        if (entry.isIntersecting) widget.dataset.state = entry.target.dataset.widget;
      }
    },
    // A section counts once it reaches the middle of the screen.
    { rootMargin: '-45% 0px -45% 0px' },
  );
  for (const section of document.querySelectorAll('[data-widget]')) watch.observe(section);
}

// Times tick along like the real widget's do.
const clocks = [...document.querySelectorAll('[data-clock]')];
const label = (seconds) => {
  const minutes = Math.floor(seconds / 60);
  return minutes < 60 ? `${minutes}m` : `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
};
const started = Date.now();
setInterval(() => {
  const elapsed = Math.floor((Date.now() - started) / 1000);
  for (const clock of clocks) {
    const text = label(Number(clock.dataset.clock) + elapsed);
    clock.textContent = clock.classList.contains('w-today') ? `${text} today` : text;
  }
}, 15_000);
