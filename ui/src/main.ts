import './app.css';
import { mount } from 'svelte';
import App from './App.svelte';

// iOS zooms into every input with a font smaller than 16px when it gets focus. `maximum-scale=1`
// stops that; iOS still allows pinch-zoom with it, but Android would not, so only on iOS
// (iPadOS reports itself as a Mac with touch).
if (/iPad|iPhone|iPod/.test(navigator.userAgent) || (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1)) {
  document.querySelector('meta[name="viewport"]')?.setAttribute('content', 'width=device-width, initial-scale=1, maximum-scale=1, viewport-fit=cover');
}

// The on-screen keyboard. Android shrinks the layout itself (`interactive-widget=resizes-content`
// in index.html); iOS (and browsers without that setting) only shrink the visual viewport and pan
// it down inside the layout viewport, which hides the header and leaves the bottom of the app
// (composer, terminal keys) behind the keyboard. Resetting the page scroll is not enough: the pan
// (`offsetTop`) can stay after the keyboard is gone, or come after the resize. So `#app` is fixed
// to the visual viewport: its offset and height (app.css). Not while pinch-zoomed: then the visual
// viewport is a window into the page that the user pans around.
const vv = window.visualViewport;
if (vv) {
  const root = document.documentElement;
  const sync = () => {
    if (vv.scale > 1.01) {
      root.style.removeProperty('--app-height');
      root.style.removeProperty('--app-top');
      return;
    }
    if (window.scrollY) window.scrollTo(0, 0);
    root.style.setProperty('--app-height', `${vv.height}px`);
    root.style.setProperty('--app-top', `${vv.offsetTop}px`);
  };
  vv.addEventListener('resize', sync);
  vv.addEventListener('scroll', sync);
  sync();
}

mount(App, { target: document.getElementById('app')! });
